"""Embedded container-side auth-file transfer, anchored to open directory FDs."""
import contextlib
import os
import re
import stat
import sys


DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
COMPLETE_FILE = ".agentcage-copy-complete"


def relative_parts(value):
    parts = value.split("/")
    if not value or any(part in ("", ".", "..") or re.fullmatch(r"[A-Za-z0-9_.-]+", part) is None for part in parts):
        raise ValueError("unsafe allowlisted relative path")
    return parts


def keep(stack, descriptor):
    stack.callback(os.close, descriptor)
    return descriptor


def open_root(stack, value):
    if not value.startswith("/") or "\x00" in value:
        raise ValueError("auth roots must be absolute")
    components = value.split("/")[1:]
    if value != "/" and any(part in ("", ".", "..") for part in components):
        raise ValueError("unsafe auth root")
    descriptor = keep(stack, os.open("/", DIRECTORY_FLAGS))
    for component in components if value != "/" else ():
        descriptor = keep(stack, os.open(component, DIRECTORY_FLAGS, dir_fd=descriptor))
    return descriptor


def parent(stack, root, parts, create=False):
    descriptor = root
    for component in parts[:-1]:
        try:
            child = os.open(component, DIRECTORY_FLAGS, dir_fd=descriptor)
        except FileNotFoundError:
            if not create:
                return None
            try:
                os.mkdir(component, 0o700, dir_fd=descriptor)
            except FileExistsError:
                pass
            child = os.open(component, DIRECTORY_FLAGS, dir_fd=descriptor)
        descriptor = keep(stack, child)
    return descriptor


def regular_file(descriptor):
    info = os.fstat(descriptor)
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise ValueError("auth file must be a regular, unlinked inode")
    return info


def source_bytes(stack, root, parts, maximum):
    directory = parent(stack, root, parts)
    if directory is None:
        return None
    try:
        descriptor = os.open(parts[-1], FILE_FLAGS, dir_fd=directory)
    except FileNotFoundError:
        return None
    with os.fdopen(descriptor, "rb") as source:
        if regular_file(source.fileno()).st_size > maximum:
            raise ValueError("auth file exceeds configured size limit")
        content = source.read(maximum + 1)
        if len(content) > maximum:
            raise ValueError("auth file exceeds configured size limit")
        regular_file(source.fileno())
        return content


def destination(stack, root, parts, create):
    directory = parent(stack, root, parts, create=create)
    if directory is None:
        return None
    try:
        descriptor = os.open(parts[-1], FILE_FLAGS, dir_fd=directory)
    except FileNotFoundError:
        return directory
    try:
        regular_file(descriptor)
    finally:
        os.close(descriptor)
    return directory


def exclusive_temporary(directory, content):
    for attempt in range(100):
        name = ".agentcage-auth-" + os.urandom(16).hex()
        try:
            descriptor = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                                 0o600, dir_fd=directory)
        except FileExistsError:
            continue
        try:
            with os.fdopen(descriptor, "wb") as output:
                os.fchmod(output.fileno(), 0o600)
                output.write(content)
                output.flush()
                os.fsync(output.fileno())
            return name
        except BaseException:
            os.unlink(name, dir_fd=directory)
            raise
    raise ValueError("cannot allocate exclusive auth temporary")


def transfer(source_root, destination_root, maximum, paths, export=False):
    if type(maximum) is not int or not 0 <= maximum <= 10 * 1024 * 1024:
        raise ValueError("invalid auth size limit")
    parts = [relative_parts(value) for value in paths]
    if not parts or len({tuple(value) for value in parts}) != len(parts):
        raise ValueError("auth paths must be nonempty and unique")
    with contextlib.ExitStack() as stack:
        target = open_root(stack, destination_root)
        if export:
            try:
                os.unlink(COMPLETE_FILE, dir_fd=target)
            except FileNotFoundError:
                pass
        source = open_root(stack, source_root)
        # Read the complete input first; no credential bytes change if any later
        # path is oversized, redirected or otherwise unsafe.
        contents = [source_bytes(stack, source, value, maximum) for value in parts]
        directories = [destination(stack, target, value, content is not None) for value, content in zip(parts, contents)]
        temporaries = []
        try:
            # Stage every write before publishing any destination. File writes
            # use the bytes from opened source inodes, never a reopened path.
            for directory, content in zip(directories, contents):
                temporaries.append(exclusive_temporary(directory, content) if content is not None else None)
            for value, directory, content, temporary in zip(parts, directories, contents, temporaries):
                if directory is None:
                    continue
                if content is None:
                    try:
                        os.unlink(value[-1], dir_fd=directory)
                    except FileNotFoundError:
                        pass
                else:
                    os.replace(temporary, value[-1], src_dir_fd=directory, dst_dir_fd=directory)
                os.fsync(directory)
        finally:
            for directory, temporary in zip(directories, temporaries):
                if temporary is not None:
                    try:
                        os.unlink(temporary, dir_fd=directory)
                    except FileNotFoundError:
                        pass

        if export:
            # Publish this ephemeral receipt only after every transfer and
            # temporary cleanup succeeded. Rust never saves partial staging.
            temporary = exclusive_temporary(target, b"complete\n")
            try:
                os.replace(temporary, COMPLETE_FILE, src_dir_fd=target, dst_dir_fd=target)
            finally:
                try:
                    os.unlink(temporary, dir_fd=target)
                except FileNotFoundError:
                    pass


def main(arguments):
    try:
        export = bool(arguments and arguments[0] == "--export")
        if export:
            arguments = arguments[1:]
        if len(arguments) < 4 or not arguments[2].isdigit():
            raise ValueError("invalid auth transfer arguments")
        transfer(arguments[0], arguments[1], int(arguments[2]), arguments[3:], export=export)
    except (OSError, ValueError, OverflowError):
        # Never echo credential bytes, source contents or attacker-controlled
        # path values in a diagnostic.
        print("agentcage: auth file sync failed; check private regular files, directory paths and size limits", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
