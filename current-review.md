# Kritická recenze AgentCage

**Hodnocená revize:** `7bca4e4ae0e0d7c8586c5876543e18c676ddce01` (HEAD při zahájení i dokončení kontrol).\
**Datum:** 4. 10. 2026, Europe/Budapest.\
**Stav:** cíl NEPOTVRZEN. Implementace nebyla opravována. Tento soubor je výstup recenze, nikoli součást hodnocené revize.

Výchozí Git pracovní strom byl čistý. Recenzi vedl koordinátor a tři kritičtí subagenti: funkční správnost/architektura, terminál/web/dokumentace (následně také distribuce/testy) a bezpečnost/izolace. Známky jednotlivých oblastí vznikly samostatně; kritikům nebyla předána předchozí skóre ani autorova obhajoba. Potvrzené pozorování jiného kritika mohlo sloužit jako důkaz, nikoli jako známka. Žádná oblast nebyla zprůměrována s jinou.

## Rozsah a pevná kritéria

Rozsah vychází z uživatelského zadání a minimalistického kontraktu AGENTS.md: jeden projekt, jeden agent, rootless Podman, disposable home, volitelné existující login/env persistence. Hodnotí se i existující dokumentační web. Desktop UI, dashboardy, policy engine, secret scanning, audit log a diff/apply nejsou požadavky; jejich absence není vada. Windows/macOS nativní podpora není slíbena — Linux-only kód sám o sobě není nedostatek.

Každá oblast má stejné pevné váhy. Každé kritérium je 0–2 body, součet 0–10:

| Kritérium | Co musí být doloženo |
|---|---|
| K1 — deklarovaný kontrakt a hlavní cesta | Přísliby odpovídají chování; nový uživatel dokončí hlavní úkol. |
| K2 — okrajové případy a selhání | Chyby, přerušení, souběh a neplatné vstupy nezpůsobují tichý úspěch, nečekanou ztrátu dat nebo nejasný stav. |
| K3 — konzistence integrace | Parser, procesní kontrakt, runtime předpoklady, agenti, návody a UI spolu souhlasí. |
| K4 — ověřitelnost a regresní ochrana | Existují relevantní provedené kontroly; testy ověřují účinek, ne pouze přítomnost argumentu. |
| K5 — provozní použitelnost a údržba | Diagnostika, obnovitelnost, předvídatelné vydání a přiměřená jednoduchost; u webu také přístupnost a ovládání. |

Kotvy: 0 = nesplněno/chybí důkaz; 1 = částečně splněno s podstatným nedostatkem; 2 = splněno a doloženo. Mezibody vyjadřují rozsah konkrétních nedostatků, nikoli procento coverage. Neprovedený test není úspěšný test. Známky při chybějících živých kontrolách jsou předběžné.

**Podmínka cíle:** nejméně **9,5/10 v každé z osmi oblastí**, žádný otevřený P0/P1, žádná neověřená zásadní část kontraktu, provedené acceptance/regresní scénáře a nové nezávislé hodnocení po opravách. Průměr ani samotných 14 zelených testů podmínku nesplní. Kritéria a rozsah se při opravách nesmí změnit jen kvůli zvýšení známky.

Priority: P0 = okamžitá kritická blokace s doloženým dopadem; P1 = nebezpečné/destruktivní chování nebo blokace hlavní cesty; P2 = významná spolehlivost, integrace, použitelnost či důkazní mezera; P3 = menší údržbová slabina. U neověřených kandidátů je priorita **prioritou ověření**, nikoli potvrzenou závažností zranitelnosti.

## Nezávislá hodnocení

| Oblast | K1 | K2 | K3 | K4 | K5 | Celkem | Stav důkazů |
|---|---:|---:|---:|---:|---:|---:|---|
| Funkční správnost CLI/runtime | 1,5 | 0,6 | 1,1 | 0,8 | 1,3 | **5,3/10** | Skutečná binárka + fake Podman; skutečný container neověřen. |
| Architektura a udržovatelnost | 1,8 | 1,1 | 1,3 | 0,7 | 1,6 | **6,5/10** | Zdroj a vybrané skutečné filesystem operace; agent parity v image neověřena. |
| Bezpečnost a izolace | 1,4 | 1,0 | 1,4 | 0,5 | 1,2 | **5,5/10, předběžné** | Částečný statický audit; runtime izolace a kandidáti neověřeni. |
| Terminálové UI/UX | 1,5 | 0,8 | 0,8 | 0,7 | 1,2 | **5,0/10** | Help/source, Bash probe, skutečný CLI s mock runtime; PTY/Zsh/Fish neověřeny. |
| Veřejná dokumentace | 1,4 | 1,2 | 0,9 | 0,7 | 1,6 | **5,8/10** | Přísliby porovnány se zdrojem; vybrané instalační kroky reprodukovány. |
| Web UI/UX a přístupnost | 1,0 | 0,6 | 0,8 | 0,5 | 0,7 | **3,6/10, předběžné** | HTML/CSS a skutečný JS v izolovaném VM; render/mobile/screen reader neověřeny. |
| Instalace, distribuce a obraz | 0,8 | 0,4 | 0,8 | 0,5 | 0,7 | **3,2/10** | Přesný HEAD selhává na Linux CRLF; další větve zkoumány v označených diagnostických kopiích. |
| Testy, CI a regresní ochrana | 1,2 | 0,7 | 0,3 | 0,5 | 0,6 | **3,3/10** | 14/14 unit tests; žádný Cargo CI ani živý Podman smoke. |

Skóre odrážejí konkrétní nálezy níže. Funkční oblast sráží zejména RT-01 až RT-06; architekturu RT-05/06, AR-02/03 a QA-01; bezpečnost nedoložený účinek izolace, SEC-N1/N2, DOC-01 a distribuce měnitelných vstupů. CLI sráží completions, help, diagnostika a streamy; dokumentaci nepravdivé přísliby a rozbité cesty; web onboarding, keyboard/ARIA a chybové stavy. Distribuci sráží nefunkční přesné entry points, nesprávný bootstrap/output kontrakt a pohyblivé image vstupy; QA nedoložený lifecycle, chybějící CI a kompatibilitní matice. Pozitivní důkazy: malá Rust dependency surface, lockfile, embedded Dockerfile, funkční kompilace, základní unit sada a pravdivější README/SECURITY. Jedna příčina může zasahovat více oblastí, ale je evidována jen jednou jako nález.

## Provedené kontroly a jejich limity

- Staticky přečteny `src/main.rs`, celé `src/lib.rs`, `src/agent.rs`, manifest/lock, Dockerfile, oba shell skripty, jediný Pages workflow, AGENTS.md, README, SECURITY, LOGIN_PERSISTENCE, TUTORIAL, TODO, init, veřejné HTML/Markdown a relevantní tvrzení v historickém transcriptu. Ignorované build artefakty, Git historie a tajné uživatelské soubory nejsou předmětem auditu.
- WSL Ubuntu 24.04, Linux x86_64, UID/GID 1000: dočasný Rust 1.99.0 (stable, 2026-10-01) mimo projekt, bez změny uživatelského PATH/profile. `cargo test --locked`: **14 prošlo, 0 selhalo**; debug `cargo build --locked` prošel. Cargo target/cache byly mimo checkout. Chyběl Podman, crun, Zsh a Fish.
- Přesná debug binárka s izolovaným fake Podmanem: skutečně ověřeny argv, process status, cleanup a host copy/finalize operace. Žádný skutečný agent, skutečný token ani container nebyl spuštěn. Výsledky mocku nepotvrzují kernel izolaci nebo upstream OAuth.
- Přesný `git archive HEAD` rozbalen mimo projekt: `./install.sh --help` i `./scripts/package.sh` **exit 127** kvůli CRLF shebangu. Následně JEN diagnostické kopie obou `.sh` normalizovány na LF, aby šlo odděleně zkoumat další chyby. Tyto kopie nejsou opravou implementace ani důkazem funkčnosti původního HEAD.
- Diagnostická LF kopie: release build/package, explicitní `install.sh --no-build --bin-dir <isolated>` a nainstalované `ac --help` prošly. Cargo.toml zůstal přesný, tedy CRLF; název balíčku obsahoval `\r`. Dokumentované `cd agentcage-*` skutečně skončilo `too many arguments`.
- Skutečný Bash completion string extrahován a vyvolán v čistém Bash: `_init_completion: command not found`. JS ze stránky vyvolán v izolovaném Node VM: clipboard denied/unavailable bez uživatelské recovery informace.
- **NEOVĚŘENO:** image build/start na skutečném Podmanu, spuštění všech čtyř agentů, OAuth a refresh, symlink bezpečnost při návratu stagingu, skutečné skrytí `.agentcage`, disposable home, SELinux, rootless crun/runc, cgroups enforcement, signal/PTY, Zsh/Fish, skutečný web render/kontrast/reflow/mobile/zoom/keyboard/screen reader a živé browser permissions.

Chrome nebyl dostupný a IAB odmítl `file://` bezpečnostní politikou; odmítnutí nebylo obcházeno. Bezpečnostní subagent byl zastaven službou pro možný kyberbezpečnostní risk ještě před vytvořením či spuštěním reprodukce. Pokračovalo pouze bezpečné předání statických podkladů. Standard scan byl uzavřen s `coverage.completeness=partial`, `findings=[]` a dvěma deferred otázkami. **Prázdný seznam findings není potvrzení bezpečnosti.** Daybreak přístup účtu není udělen, chráněné výsledky nemusí být zobrazitelné; toto nebránilo běžnému file reportu.

### Evidence mimo repozitář

- [Runtime harness](C:/Users/dubst/AppData/Local/Temp/agentcage-runtime-review-20261004/probe_runtime.py) a [runtime výsledky](C:/Users/dubst/AppData/Local/Temp/agentcage-runtime-review-20261004/runtime-report.json).
- [Release harness](C:/Users/dubst/AppData/Local/Temp/agentcage-release-review-20261004/probe_release.py) a [release výsledky](C:/Users/dubst/AppData/Local/Temp/agentcage-release-review-20261004/release-report.json).
- [Canonical security report](C:/Users/dubst/.codex/state/plugins/codex-security/scans/AgentCage/7bca4e4ae0e0d7c8586c5876543e18c676ddce01_20261004T153903Z_lw1beb8o/report.md) a [částečné coverage](C:/Users/dubst/.codex/state/plugins/codex-security/scans/AgentCage/7bca4e4ae0e0d7c8586c5876543e18c676ddce01_20261004T153903Z_lw1beb8o/coverage.json). Scan ID `7ba0e378-feb0-465c-81cb-1741e49cc5c9`; token measurement unavailable.

Dočasné evidence mohou být později smazány; jejich podstatné vstupy, pozorování a acceptance jsou proto zachyceny níže. Zdrojové odkazy `src/...:řádek` se vždy vztahují k výše uvedenému SHA, root `C:/Users/dubst/Documents/GitHub/AgentCage`.

## Prioritní nálezy — runtime a persistence

### RT-01 — P1: `clean` může násilně ukončit živou i cizí práci

**Důkaz:** `src/lib.rs:570–596`, zejména filtr `name=agentcage`, `ps -a` a `rm -f`; `build_run_argv:1482` nemá ownership label. Help `src/lib.rs:1899` slibuje úklid leftover containers. `cwd` je ignorováno.

**Reprodukce:** fake `ps` vrátí `unrelated-agentcage-backup` a `agentcage-live`; skutečné `ac clean` následně zavolá `rm -f` pro oba. Scénář `clean_rm_failure_and_names` v runtime evidence. Očekává se úklid vlastních zastavených zbytků, nikoli ukončení aktivní práce. Dopad na skutečný container nebyl riskován; výběr a destruktivní argv jsou potvrzeny. Podman name filter přijímá regex a `ps -a` zahrnuje i running [dle primární dokumentace](https://docs.podman.io/en/latest/markdown/podman-ps.1.html).

**Doporučení a postup opravy:** 1. Označit vytvářené kontejnery jednoznačným interním labelem. 2. Definovat globální/projektový scope v help. 3. Uklízet pouze vlastní zastavené kontejnery podle ID, bez `-f`. 4. Staré kontejnery bez labelu řešit explicitně, nikoli širokou shodou názvu. 5. Propagovat chyby dle RT-02.

**Ověření:** vlastní exited se odstraní; vlastní running, cizí podobně pojmenovaný i cizí se stejným prefixem přežijí. Přechod exited→running mezi seznamem a odstraněním nesmí ukončit proces. Dva současné cleanupy musí mít pravdivý výsledek. Integrační test smí používat jen vlastní označené disposable fixtures.

### RT-02 — P2: `clean` hlásí úspěch při selhání `ps` i `rm`

**Důkaz:** `src/lib.rs:581–595` nekontroluje status `ps` a zahazuje výsledky `rm`.

**Reprodukce:** `clean_ps_failure`: mock `ps` status 125 → skutečné `ac` status **0**, text `nothing to clean`. `clean_rm_failure_and_names`: oba mock `rm` status 125 → `ac` **0**, závěrečné `done`. Očekávaný výsledek je chyba, nikoli potvrzení prázdného/uklizeného stavu.

**Doporučení a postup opravy:** 1. Ověřit status každého příkazu. 2. Odlišit prázdný úspěšný seznam od chyby a uvést omezenou stderr diagnostiku. 3. Evidovat úspěšné/neúspěšné odstranění. 4. Dílčí failure vrátit callerovi nenulovým kódem; případné pokračování další položkou nesmí být prezentováno jako plný úspěch.

**Ověření:** ps failure nesmí hlásit empty; rm failure nesmí hlásit úspěšné done; smíšený seznam musí mít nenulový závěr a rozlišitelné výsledky. Regrese: prázdný úspěšný seznam a plně úspěšný cleanup stále vracejí 0.

### RT-03 — P2: návratové kódy procesu se ztrácejí a 130 je úspěch

**Důkaz:** `src/lib.rs:134–140`, `1655–1665`.

**Reprodukce:** skutečný CLI s mock child 42 vrátí **1**; child 130 vrátí **0** bez chyby. Scénáře `exit_42`, `exit_130`. Přerušená session tak dovolí pokračování příkazu po `&&`. Podman dokumentuje výsledné child kódy a vlastní 125–127 [v exit status kontraktu](https://docs.podman.io/en/latest/markdown/podman-run.1.html#exit-status).

**Doporučení a postup opravy:** 1. Oddělit vnitřní chybu wrapperu od výsledku child. 2. Zachovat child exit až do `run`. 3. Odstranit převod 130 na `Ok(())`. 4. Definovat signal a současné selhání finalizace; cleanup provést před návratem a uvést oba důvody bez maskování primárního výsledku.

**Ověření:** child 0/1/42/125/126/127/130; `ac … && marker` pokračuje pouze při skutečném úspěchu. Ověřit cleanup/login finalize pro všechny výsledky a skutečný Ctrl-C/TERM v PTY. Signály jsou dosud neověřené.

### RT-04 — P2: pipe stdin se nepředává, shell vždy vynucuje TTY

**Důkaz:** `src/lib.rs:1503–1504`, `1625`, `1846`: agent dostane `-it` pouze podle stdin TTY, jinak ani `-i`; shell dostane `-it` vždy.

**Reprodukce:** `printf 'probe\n' | ac codex --no-login -- exec -` v mocku zachycuje `interactive=false`; `printf 'exit\n' | ac shell` zachycuje `tty=true`. Konstrukce argv je potvrzena, skutečný Codex pipe není. Podman bez `-i` vstup uzavírá; pro pipe uvádí `-i` bez `-t` [v run dokumentaci](https://docs.podman.io/en/latest/markdown/podman-run.1.html#interactive-i).

**Doporučení a postup opravy:** 1. Oddělit připojení stdin a alokaci TTY. 2. Foreground input umožnit přes `-i`. 3. `-t` volit podle vhodných terminálových deskriptorů; shell s pipe jej nesmí automaticky dostat. 4. Zachovat oddělené streamy a opravit banner dle UX-04.

**Ověření:** PTY, stdin pipe, `/dev/null`, stdout soubor, oddělené stderr; neškodný container `cat` musí vracet přesné vstupní bajty. Redirectovaný stdout nesmí obsahovat TTY control chars. Mock i skutečný podporovaný Podman musí projít.

### RT-05 — P1: souběžné session přepisují nové přihlášení starým snapshotem

**Důkaz:** `src/lib.rs:876–912`, `1292–1342`: staging vzniká ze sdíleného per-agent store, finalize jej přepisuje bez konfliktové kontroly.

**Reprodukce:** dvě skutečné CLI session s koordinovaným fake child, umělé tokeny. A i B načtou `old`; A uloží `new`; B končí později a uloží `old`. Oba exit 0. Evidence `concurrent_auth_stale_overwrite`; nepoužity skutečné credentials. Očekává se zachování novějšího refresh/login, nikoli bezpodmínečné last-writer-wins.

**Doporučení a postup opravy:** 1. Zavést krátké per-agent transakce pod lockem a generaci/fingerprint snapshotu. 2. Pod stejným lockem načítat/finalizovat/mazat store. 3. Nezměněný staging nesmí přepsat mezitím změněný store. 4. Dvě nezávislé změny vyhodnotit jako konflikt, nikoli tiše vrátit starý token. 5. Lock/generaci držet mimo mazaný credential strom. Netřeba session historie, policy engine ani databáze.

**Ověření:** A refresh/B beze změny; obě session odlišný refresh; logout versus refresh; obě pořadí dokončení; různým agentům neblokovat celý běh. Testy používat deterministické bariéry, ne náhodné sleeps.

### RT-06 — P1: úspěšné `login rm` může ukončení staré session zvrátit

**Důkaz:** `src/lib.rs:729–761`, `1325–1329`.

**Reprodukce:** otevřená login-enabled session, `ac login rm codex` vrací 0 a soubor zmizí; po dokončení staré session se stejné `old` credentials znovu vytvoří, session exit 0. Evidence `login_remove_during_run` a `removed_login_resurrection`. Očekává se, že odstranění přetrvá bez nového vědomého přihlášení. Samotný lock pouze kolem mazání nestačí.

**Doporučení a postup opravy:** společně s RT-05. 1. `login rm` pod lockem změní generaci/revokaci a odstraní data. 2. Finalize se starou generací nesmí store obnovit. 3. Přeskočenou persistence/konflikt oznámit. 4. Jasně určit podmínky pro nové uložení po novém loginu.

**Ověření:** delete během běhu, mezi kontrolou a kopírováním, dvě otevřené session, následný nový login a běžné mazání bez session. `login list` po ukončení starých session nesmí ukázat odstraněný login. Odstranění souboru není odvolání tokenu u poskytovatele; tvrdit pouze lokální odstranění.

### RT-07 — P2: deklarované runtime požadavky nepostačují

**Důkaz:** `src/lib.rs:1513–1521`, `1695–1714`; preflight ověřuje rootless, ale argv vyžaduje `keep-groups` a resource limits. README/TUTORIAL požadují Linux + rootless Podman.

**Reprodukce/ověření:** mock zachytil požadované flags, preflight OCI runtime/cgroups neexistuje. Podman uvádí `keep-groups` jako crun-only a omezení rootless cgroups v1 [v primární dokumentaci](https://docs.podman.io/en/latest/markdown/podman-run.1.html#group-add-group-keep-groups). Skutečné runc/cgroups-v1 spuštění je NEOVĚŘENO, nejde o naměřený start failure.

**Doporučení a postup opravy:** 1. Stanovit minimální podporované verze, OCI runtime a cgroups. 2. Z `podman info` ověřit předpoklady používaných ochran. 3. Nepodporovanou konfiguraci odmítnout s akčním vysvětlením před spuštěním. 4. Sladit requirements, help a CI matrix; ochrany tiše nesnižovat.

**Ověření:** rootless crun/cgroups v2 projde; unsupported matrix skončí přesnou diagnostikou před agentem. Skutečně ověřit enforcement CPU/memory/PID, ne jen flags.

## Architektura

### AR-02 — P3: agent metadata mají několik ručně synchronizovaných seznamů

**Důkaz:** `src/agent.rs:1`, `src/lib.rs:235`, `915`, `1873`, `1939`, `2001`, `2102`, Dockerfile. AGENTS.md sám požaduje synchronizované změny. Aktuální chybu parity netvrdíme — je to doložené strukturální riziko při přidávání agentů.

**Doporučení a postup opravy:** 1. Malý interní descriptor pro CLI name/binary/auth paths nebo alespoň parity testy stávajících seznamů. 2. Parser a seznamy generovat z descriptoru, kde to zjednoduší kód. 3. Ruční completion šablony klidně ponechat, ale ověřit jejich parity. 4. Image smoke spustí všechny deklarované binárky. Nepřidávat framework/config/policy engine; samotné rozdělení velkého lib.rs není podmínkou cíle.

**Ověření:** pro každý agent resolve, parse, help/completion nabídka a auth konfigurace; image smoke verifikuje reálné binary start. Přidání/odebrání agenta nesmí zanechat nefunkční položku. Regresní dluh AR-01 je sloučen do QA-01.

### AR-03 — P2: JSON heuristika mění detekci loginu podle whitespace

**Důkaz:** `src/lib.rs:1218–1220`, `1271–1288`.

**Reprodukce skutečným `ac login list`:** `{"accounts":[]}` → opencode není uveden; ekvivalentní `{"accounts":[ ]}` → opencode je uveden; `{"accounts":false}` jej také uvádí. Runtime evidence `empty_accounts_*`. Whitespace nekonzistence je potvrzena; aktuální upstream schema/OAuth nejsou ověřeny.

**Doporučení a postup opravy:** 1. Ověřit skutečné upstream auth kontrakty. 2. Validovat relevantní JSON strukturu a typy, nikoli textové prefixy. 3. Zachovat limit velikosti a odmítání neplatných vstupů. 4. Respektovat zákaz serde/config frameworku: zvolit omezený parser existujících auth tokenů nebo jiný spolehlivý jednoduchý kontrakt. Pouhé odstranění whitespace ze všeho textu poškodí řetězce.

**Ověření:** ekvivalentní whitespace varianty stejný výsledek; empty array/object, null/false, nevalidní JSON, správný neprázdný dokument, stejnojmenný nested key, escapované uvozovky a whitespace uvnitř stringu. Nedokládat z toho platnost tokenu u poskytovatele.

## Bezpečnostní otázky, které nelze uzavřít

### SEC-N1 — P1 priorita ověření: import návratového stagingu a rodičovské cesty

**Stav:** NEOVĚŘENO, kandidát, nikoli potvrzená zranitelnost. `src/lib.rs:1305–1329` kontroluje leaf metadata a následně čte/kopíruje zdroj podle cesty. Nebyla dokončena nezávislá validace bezpečnosti všech rodičovských komponent ani časových závodů. Reprodukce nebyla vytvořena/spuštěna kvůli zastavení bezpečnostního subagenta.

**Doporučený bezpečný postup:** 1. Samostatně ověřit, že import přijímá pouze allowlisted běžné soubory dosažitelné uvnitř vlastního staging rootu. 2. Projít všechna čtení metadata/content/copy, nejen leaf check; odmítat symlink rodiče a změny cesty mezi kontrolou a otevřením. 3. Pokud kontrola selže, použít otevření vůči ověřenému directory descriptoru a nepovolit následování symlinků v žádné komponentě. 4. Validovat size/content na otevřeném souboru, ne jiném později znovu otevřeném path. 5. Chybu importu nezamlčet a bezpečně uklidit staging.

**Podmínka ověření:** autorizovaný nezávislý reviewer, pouze izolované umělé soubory a markery; běžný vnitřní soubor se uloží, symlink leaf/parent/outside path a souběžná výměna cesty se odmítnou, vnější fixture se nečte ani nepřepíše. Nepoužívat reálný host HOME či credentials. Bez provedených výsledků otázka zůstane deferred.

### SEC-N2 — P1 priorita ověření: skutečně prázdný shadow `.agentcage`

**Stav:** konstrukce mountu je potvrzena, dopad na reálném runtime **NEOVĚŘEN**. `src/lib.rs:1525–1527` vytváří nested `type=tmpfs` bez explicitního `notmpcopyup`. Aktuální Podman dokumentace uvádí default copyup i z rodičovského bind mountu; pro prázdné překrytí požaduje `notmpcopyup` [v mount kontraktu](https://docs.podman.io/en/latest/markdown/podman-run.1.html#mount-type-type-type-specific-option). To je konkrétní rozpor s důkazem, který nynější test `src/lib.rs:2189–2204` poskytuje; test pouze najde substring tmpfs. Kompatibilitu konkrétních podporovaných verzí jsme neověřili.

**Doporučení a postup:** 1. Fixovat podporované runtime verze a ověřit jejich tmpfs semantics. 2. Výslovně nastavit prázdné překrytí (`notmpcopyup`, pokud je správným podporovaným mechanismem). 3. Rozšířit argv test na úplnou konfiguraci. 4. Dodat skutečný sentinel test skrytí a nepřenosu obsahu, včetně persistent env. 5. Teprve podle výsledků potvrdit tvrzení v SECURITY/README.

**Ověření:** v izolovaném projektu vytvořit neškodný marker uvnitř host `.agentcage`; neškodný container příkaz jej nesmí vidět a vytvoření uvnitř overlay nesmí změnit host marker/stav. Default, `--no-login` i `--env`, různé podporované verze/SELinux. Zkontrolovat i větší host stav vůči 64M tmpfs. Zdrojová dokumentace sama nestačí k potvrzení kernel účinku.

Síť, writable repo, dostupnost vybraných tokens při login-enabled run a persistent home s `--env` jsou deklarované vlastnosti, ne samy o sobě nové zranitelnosti. Rozpor veřejných příslibů je DOC-01; supply-chain reproducibility je DIST-05; chybějící izolace testy QA-02.

## Terminálové UI/UX

### UX-01 — P2: help nevysvětluje passthrough a výluku voleb

**Důkaz/reprodukce:** `src/lib.rs:1886`, `1902` versus parse `257–258` a runtime `521`. `ac codex --help` je wrapper help; agent help je `ac codex -- --help`. `ac codex --model …` selže jako unknown wrapper option. Help neuvádí význam `--` ani nemožnost kombinovat `--no-login` a `--env`.

**Doporučení a postup:** 1. Popsat `ac <agent> [--env NAME | --no-login] [-- <agent args…>]`. 2. Přidat agent-help/raw-option příklady. 3. Chyba unknown agent option má nabídnout passthrough tip. 4. Sjednotit README, tutorial a help.

**Ověření:** nový uživatel bez README odvodí správné agent help a volební pravidlo. Parser regrese: unknown wrapper option, zachování args za `--`, zakázaná kombinace a duplicita env.

### UX-02 — P2: propagované shell completions nemají funkční čistý setup

**Důkaz:** `src/lib.rs:1943`, `2097`, README `202`, TUTORIAL `158`. Skutečný extrahovaný Bash skript v `bash --noprofile --norc` při `_ac_completions` hlásí `_init_completion: command not found`, žádné kandidáty. Závislost na bash-completion není uvedena. Zsh při přímém source neregistruje `compdef _ac ac`, končí `_ac`; tento díl je statický, Zsh runtime neověřený. Bash/Fish také nemají nabídku hodnot pro `ac completions `.

**Doporučení a postup:** 1. Zvolit jeden dokumentovaný setup kontrakt každého shellu. 2. Bash helper odstranit jednoduchou vlastní inicializací nebo prokazatelně zajistit jeho dependency/setup. 3. Zsh skutečně zaregistrovat pro source workflow, případně změnit návod na `fpath`/compinit instalaci. 4. Doplnit values/subcommands a agent wrapper volby, zastavit wrapper completion za `--`. 5. Přidat čisté shell probes.

**Ověření:** v čistém Bash/Zsh/Fish použít přesně příkaz z dokumentace, bez stderr; `ac c`, `ac completions `, `ac login rm `, `ac env rm ` a `--env` mají správné kandidáty. Po `--` nesugerovat wrapper options. Fish/Zsh nesmějí být prohlášeny za úspěšné jen syntax kontrolou.

### UX-03 — P2: preflight zahazuje Podman příčinu chyby

**Důkaz:** `src/lib.rs:1734–1758` ignoruje exit status/stderr pro version a info.

**Reprodukce k provedení:** mock `podman version/info` vrátí 125, empty stdout a popis chyby ve stderr. Zdroj předává pouze `could not determine Podman version` nebo `unexpected podman rootless value: ''`; tento konkrétní probe zatím nebyl spuštěn. Zahazování stderr/status je staticky doloženo.

**Doporučení a postup:** 1. Nejdřív kontrolovat status a uvést omezenou stderr příčinu. 2. Až při success validovat formát hodnoty. 3. Stejný procesní kontrakt aplikovat na ostatní probes. 4. Z chyb musí být jasný další krok, ne domnělá neplatná hodnota.

**Ověření:** status125, empty success, invalid value, valid true/false; chybová diagnostika rozliší service/permissions failure od neplatné odpovědi, bez citlivého výstupu.

### UX-04 — P2: wrapper banner znečišťuje stdout agenta

**Důkaz:** `src/lib.rs:537`, `793–800` používá `println!` před agentem. Skutečná binárka v `stdin_pipe` evidence vypíše AgentCage/Workspace/Home před mock JSON. Strojový parser tak nemůže použít stdout přímo. Živý JSON output konkrétního upstream agenta nebyl testován.

**Doporučení a postup:** 1. Status banner agenta/shellu přesměrovat na stderr. 2. Zachovat interaktivní čitelnost a přesné forwarding streamy. 3. Definovat stdout kontrakt v README. 4. Ověřit spolu s RT-04.

**Ověření:** fake runtime známý JSON → stdout byte-for-byte pouze JSON, banner jen stderr; poté neškodný skutečný container command. Regrese: v terminálu je stav stále viditelný. UX-05 je deduplikován do RT-01/02.

## Dokumentace a veřejné přísliby

### DOC-01 — P1: hero slibuje nepřístupné credentials a žádné stopy

**Důkaz/reprodukce čtením:** `docs/index.html:7`, `471`, `docs/index.md:5` proti README `145–150`, LOGIN_PERSISTENCE `22–29`, `89`. Default run kopíruje auth agentovi, store přežívá, projekt je writable a síť host. Historický transcript `deepseek chat inside cage.md:59`, `82` obsahuje absolutní tvrzení o nemožnosti úniku, které nejsou důkazem. Container deletion neodstraní změny projektu, image ani login store. Riziko je nesprávné rozhodnutí uživatele podle první prezentace.

**Doporučení a postup:** 1. Hero/meta/Markdown omezit na přesnou hranici nemountování host home. 2. Viditelně uvést writable projekt, síť a implicitní auth persistence. 3. Zobrazit `--no-login` a limity. 4. Transcript označit jako historické neověřené výroky, nikoli návod/bezpečnostní důkaz. 5. Sjednotit text se SECURITY a login dokumentací; nevykládat containment jako VM.

**Ověření:** žádný veřejný text nemá nepodmíněné credentials-out-of-reach/no-traces/nemožnost-úniku tvrzení. Čerstvý kritik pouze z úvodu správně popíše, co agent dostává, co může změnit, kam může komunikovat a co přetrvává. SEC-DOC je stejná příčina, ne další nález.

### DOC-02 — P2: release příklad `cd agentcage-*` selže po rozbalení

**Důkaz:** README `64–65`, TUTORIAL `32–33`, package `12`, `35`.

**Reprodukce potvrzena:** archive a extrahovaný stejnojmenný adresář existují současně; Bash `cd agentcage-*` dostane dva argumenty a vrátí `too many arguments`. Potvrzeno nad skutečným diagnostickým release package, nezávisle na CRLF jméně. Při více releases je glob ještě méně jednoznačný.

**Doporučení a postup:** 1. Uvést konkrétní archive/target a odpovídající jednoznačný adresář. 2. Directory-only glob použít jen s výslovnou podmínkou jednoho adresáře. 3. Sjednotit README/TUTORIAL/web a přidat cestu ke stažení vydání.

**Ověření:** přesný copy/paste blok projde v čisté složce s archive i extrahovaným dir až po installer; s více releases musí mít jednoznačný výběr. Regrese: source install postup zůstane samostatný.

### DOC-03 — P2: tutorial source install má shell placeholder a chybné case

**Důkaz:** TUTORIAL `40–41`: `git clone <repo-url>` a `cd agentcage`, zatímco skutečný repo název je `AgentCage`.

**Reprodukce k provedení:** literal placeholder je shell redirection syntax; po dosazení URL z README Git bez explicitního destination vytvoří `AgentCage`, Linux `cd agentcage` nenajde. Nepoužit síťový clone; zdroj a case kontrakt jsou přímo doložené.

**Doporučení a postup:** 1. Konkrétní správná URL a explicitní clone destination. 2. Odpovídající `cd`. 3. Uvést Git/Rust pro source/quick-install větev. 4. Kontrolovat tutorial copy/paste při release.

**Ověření:** vytištěný postup je syntakticky platný bez ručního nahrazování a projde na case-sensitive Linux fixture. Nový uživatel nemusí opravovat příkazy z hlavy.

## Webové UI/UX

### WEB-01 — P2: Quick Start přeskočí projekt a povinný image init

**Důkaz:** `docs/index.html:543`, `558`, `docs/index.md:12` versus `src/lib.rs:526`. Po instalaci rovnou nabízí `ac codex`, chybí `cd` do cílového projektu a `ac` pro build image.

**Reprodukce očekávaná dle zdroje:** na čistém hostu bez image agent command skončí `image ... not found. run 'ac' first`; konkrétní browser/Podman journey neproveden. Uživatel může také omylem mountnout instalační složku.

**Doporučení a postup:** 1. Úplná sekvence `cd /path/to/project`, `ac`, agent. 2. Vysvětlit první network build a požadavky Git/Cargo pro curl/source installer. 3. Přidat recovery odkaz a variantu `--no-login`. 4. Udržet copy buttons konzistentní s textem.

**Ověření:** nový uživatel na čistém podporovaném hostu dokončí pouze webový postup; mountuje správnou cestu, image se vytvoří a zvolený agent začne. Nejprve odstranit DIST-01.

### WEB-02 — P2: Copy tlačítka mají neviditelný keyboard focus

**Důkaz:** `docs/index.html:243–262`, `523`, `562`: `opacity:0`, zobrazení jen na hover; chybí focus/focus-within. Přístupná jména jsou shodné `Copy`. Statický CSS/HTML důkaz; skutečný keyboard/touch běh neproveden.

**Doporučení a postup:** 1. Viditelný default nebo zobrazení na focus-within i touch. 2. Jednoznačný focus-visible. 3. Přístupná jména popisující konkrétní příkaz. 4. Dostatečný touch target a test při zoomu.

**Ověření:** Tab ukazuje každý ovládací prvek, screen reader button list je rozlišuje, myš/touch/keyboard umějí získat příkaz. Po kopírování focus zůstane zachován.

### WEB-03 — P2: hlavní heading není přístupný a install přepínače nemají stav

**Důkaz:** `docs/index.html:83`, `465`, `513`, `653`: jediný h1 je display:none; install buttons přepínají class, bez deklarace vybrané volby a vazby na panel.

**Doporučení a postup:** 1. Viditelný brand udělat skutečným h1, odstranit skrytý duplicate. 2. Vybrat kompletní tablist/tab/tabpanel kontrakt včetně keyboard šipek nebo jednodušší pojmenovaná tlačítka s aria-pressed. 3. Synchronně aktualizovat state/visibility/vazby. 4. Dekorativní SVG označit jako dekoraci.

**Ověření:** právě jeden hlavní heading v accessibility tree; screen reader oznámí vybraný způsob instalace a panel; celé přepnutí funguje klávesnicí. Skutečný screen-reader/browser test je nyní NEOVĚŘEN.

### WEB-04 — P2: clipboard failure nemá recovery ani viditelné sdělení

**Důkaz/reprodukce skutečného JS v VM:** `docs/index.html:640–648`. Promise rejection ponechá `Copy`, pouze console.error; chybějící navigator.clipboard vyvolá TypeError. Browser permission test neproveden.

**Doporučení a postup:** 1. Guard dostupnosti API, try/catch a rejection handling. 2. Inline success/failure status v přístupné live oblasti. 3. Při failure umožnit snadný ruční výběr/kopírování textu s konkrétním vysvětlením. 4. Reset success stavu bez ztráty focus.

**Ověření:** granted/denied/unavailable API; žádná uncaught exception, uživatel pozná výsledek a při failure získá příkaz ručně. Provedení VM testu doplnit skutečným browserem.

### WEB-N1 — P2 priorita ověření: mobile/zoom ořez dlouhých příkazů

**Stav:** NEOVĚŘENO. `docs/index.html:44`, `54`, `156`, `217`, `232`, `442`: skládání paddingů, flex bez wrapu, chybějící min-width/wrap pravidlo pro command a skrytý body horizontal overflow vytvářejí konkrétní riziko. Bez renderu není potvrzen vizuální defect ani kontrast.

**Postup ověření a případné opravy:** 1. Všechny install panely na 320/360/390 px a 200–400% zoom. 2. Měřit bounding boxes a přístup k celému curl/Git příkazu i Copy. 3. Při potvrzení použít responsive padding, min-width:0, wrapping nebo lokální horizontal scrolling; neskrývat ztrátu obsahu overflowm těla. 4. Ověřit touch a keyboard recovery.

**Acceptance:** žádný ztracený text/ovládání, vše lze vidět a získat bez horizontálního scrollování celé stránky. Vizuální estetiku, reflow a kontrast bez provedení neznámkujeme jako ověřené.

## Instalace, distribuce a testy

### DIST-01 — P1: tracked shell skripty nelze na Linuxu spustit

**Důkaz:** `install.sh:1–2`, `scripts/package.sh:1–2`, přesný Git archive HEAD: install 96 CRLF/96 LF, package 37 CRLF/37 LF; executable Git modes jsou správné 100755. Chybí LF enforcement.

**Reprodukce potvrzena:** v Linux snapshotu `./install.sh --help` a `./scripts/package.sh` exit127 `/usr/bin/env: sh\r: No such file or directory`; `sh scripts/package.sh` také selže `set: Illegal option -\r`. Postup README `./install.sh` ani package build tak nemůže začít. Na diagnostické LF kopii sh manifest CRLF navíc způsobí archive `agentcage-0.1.0\r-x86_64-unknown-linux-gnu.tar.gz` — `scripts/package.sh:10` ponechá trailing CR po sed substituci.

**Doporučení a postup:** 1. Definovat `.gitattributes` LF pro `.sh`, manifesty a další distribuované textové vstupy. 2. Normalizovat tracked blobs při zachování 100755 a ověřit diff. 3. Package version validovat jako očekávanou verzi bez whitespace/control chars, nikoli slepě přebírat sed output. 4. CI testovat `git archive`/čistý Linux checkout, ne pouze lokálně normalizovaný working tree.

**Ověření:** přesný nový archive má LF shell shebangs; `./install.sh --help`, package build, tar extraction a `--no-build` install projdou. Názvy package/target nesmějí obsahovat CR/newline. Regrese na Linux checkout i Windows checkout se stanoveným LF kontraktem. Diagnostické kopie v tomto auditu podmínku nesplňují pro původní HEAD.

### DIST-02 — P2: bootstrap při klonování ztratí už zpracované volby

**Důkaz:** `install.sh:21–47` spotřebuje args, pak `55` rekurzivně předává prázdné `$@`.

**Reprodukce:** na dočasné LF kopii mimo repo, fake Git/Cargo a umělý HOME: `sh install.sh --no-build --bin-dir <requested>`. Clone větev spustí `cargo build --release` a instaluje do default fake-home `.local/bin`, requested/ac nevznikne. Evidence `bootstrap_options_lost_after_normalization`. Původní CRLF je dřívější blocker; tato chyba je latentní po jeho odstranění.

**Doporučení a postup:** 1. Před parsováním zachovat argumenty nebo rekurzi nahradit pokračováním se stejným parsed stavem v zvoleném repo rootu. 2. Nedovolit změnu prefix/build policy při bootstrapu. 3. Cleanup cloned temp zajistit i při selhání child. 4. Explicitně určit význam `--no-build` bez existujícího package/binary; při nemožnosti vysvětlit chybu, ne tajně buildit.

**Ověření:** bootstrap/local source/package pro default, --prefix, --bin-dir, --no-build a mezery v cestě; stejné effective options a předvídatelná failure bez nečekaného buildu. Git/child failure uklidí pouze vlastní temp.

### DIST-03 — P1: piped installer může buildit cizí Cargo projekt

**Důkaz:** `install.sh:49–58` odvozuje directory z `$0` a přítomnost libovolného Cargo.toml pokládá za AgentCage. Při `bash` stdin je `$0=bash` a script_dir je aktuální pracovní složka. README `58` doporučuje tento způsob.

**Reprodukce bezpečným fixture:** normalizovaný přesný script předaný do `bash -s` v unrelated dir s Cargo.toml; mock Cargo vytvoří fixture `target/release/ac`. Installer neklonuje AgentCage, buildí/installuje fixture a oznámí úspěch. Evidence `piped_install_uses_unrelated_cargo_project_after_normalization`. Nepoužit síťový clone ani skutečný cizí build.

**Doporučení a postup:** 1. Rozlišit standalone file script a stdin bootstrap. 2. V bootstrapu vždy získat zamýšlený vlastní zdroj/package do vlastní temp složky. 3. Local větev ověřit konkrétním AgentCage source identity, ne libovolným manifestem. 4. Výstup instalace identifikovat verzí/revizí; nedělat cargo build v nahodilém cwd.

**Ověření:** prázdný dir, AgentCage checkout, cizí Cargo dir s/bez target/release/ac, path s mezerami. Všechny bootstrap výsledky instalují skutečný zamýšlený AgentCage artefakt; cizí projekt se nebuildí ani jeho binary neinstaluje. Po LF opravě testovat přesnou doporučenou příkazovou cestu.

### DIST-04 — P2: build target directory a hledaná binary se rozcházejí

**Důkaz:** `install.sh:71`, `74`, package `15`, `20`; Cargo respektuje external target configuration, skripty hledají fixed `target/release/ac`.

**Reprodukce:** LF diagnostická kopie, `CARGO_TARGET_DIR=<external>`, fake Cargo úspěšně uloží binary tam; installer skončí1 `release binary not found` v hardcoded target dir. Evidence `external_target_dir_not_found_after_normalization`. Bezpečný parametr prostředí je reálný podporovaný Cargo kontrakt; cross-target packaging navíc nebylo testováno.

**Doporučení a postup:** 1. Jednoznačně určit build output path pro build i instalaci/package (respektovat Cargo metadata/config nebo explicitně řídit target-dir). `CARGO_TARGET_DIR` je [dokumentovaný Cargo vstup](https://doc.rust-lang.org/cargo/reference/environment-variables.html), nikoli nepodporovaný hack. 2. Oddělit no-build package layout od source output. 3. Názvy target package odvozovat ze skutečně sestaveného targetu, ne slepě rustc host. 4. Chybové sdělení uvést skutečné hledané místo/target.

**Ověření:** default, external CARGO_TARGET_DIR, config target-dir a podporovaný cross target nebo jeho včasné explicitní odmítnutí. Archivovaný target a binary arch souhlasí, nepoužije se stale default binary.

### DIST-05 — P2: image/release vstupy a aktualizace nejsou reprodukovatelné

**Důkaz:** Dockerfile `1`, `19–24` Ubuntu tag, npm bez verzí, měnitelný curl installer. `src/lib.rs:1780–1803` readiness porovnává pouze Dockerfile hash. `scripts/package.sh:15`, install `71` build bez --locked; release checksums/signatures jsou stále jen TODO `25`. Nejde o zjištěnou kompromitaci závislostí.

**Reprodukce/limit:** zdroj prokazuje nefixované vstupy, ale dvě skutečná image sestavení ani drift upstreamek nebyly provedena. Stejný Dockerfile hash nerozlišuje různé stažené verze a neřeší řízené updates. Dostupnost/existence executable není důkaz jejich startu/kompatibility.

**Doporučení a postup:** 1. Stanovit podporované agent verze, Ubuntu digest a ověřené third-party installer artefakty. 2. Řízené update/rebuild pravidlo s dohledatelnou sadou verzí v release metadatech, bez zavádění session audit logs. 3. Release build --locked a jednoznačný target. 4. Vydání dodat checksums, přiměřené ověření původu a validační návod. 5. Použít stabilní obsahový hash pro image freshness; [DefaultHasher negarantuje stabilitu hashů mezi releases](https://doc.rust-lang.org/std/collections/hash_map/struct.DefaultHasher.html).

**Ověření:** dva čisté builds mají stejnou sadu vstupních verzí/digestů; každá deklarovaná agent binary skutečně startuje ve finálním read-only runtime. Změna schválené verze vyvolá rebuild; release checksum odpovídá archive a poškozený artefakt ověření odmítne. Tvrzení „reprodukovatelný bit po bitu“ lze použít jen s takovým měřením.

### QA-01 — P1: zelená unit sada nechrání potvrzené lifecycle chyby

**Důkaz:** test modul `src/lib.rs:2146–2383`, 14 testů. Pokrývá vybrané helpers, parser a argv, nikoli process statuses, clean failures, concurrency/revocation, shell pipe kontrakt nebo installer. 14/14 prošlo, zatímco runtime/release probes výše reprodukovaly vady.

**Doporučení a postup:** 1. Deterministické fake runtime probes převést do automatické regresní sady. 2. Každý proces izolovat vlastním PATH/XDG/TMP/fixture storem. 3. Souběh řídit bariérami. 4. Přidat shell-script/install/package smoke z přesného Git archive. 5. Test musí selhávat na této revizi a projít po konkrétní opravě, ne jen opisovat implementaci. Zachovat malou sadu zaměřenou na skutečné kontrakty.

**Ověření:** RT-01..06, AR-03, UX-02/04, DIST-01..04 a DOC-02 mají automatické failing-before/passing-after evidence. Nepoužívat skutečné tokens, cizí kontejnery ani user settings. Coverage procento nebylo měřeno a není zde vymyšleno. AR-01 je sloučen do tohoto nálezu.

### QA-02 — P1 důkazní mezera: chybí skutečný containment a agent smoke

**Důkaz:** `src/lib.rs:2170–2221` ověřuje strings/argv; žádný provedený skutečný Podman test. Nelze z toho potvrdit `.agentcage` skrytí, disposable home, resource limits, teardown nebo OAuth.

**Doporučení a postup:** 1. Malá opt-in Linux smoke sada s neškodnými příkazy a vlastními označenými fixtures. 2. Sentinel mount/home tests, UID/GID/private perms, read-only image root, CPU/memory/PID enforcement a teardown. 3. Spustit každý podporovaný agent help/version ve skutečném runtime; následně autorizované OAuth/refresh s účty určenými jen pro testy. 4. Verze Podman/OCI/cgroups/SELinux zaznamenat ve výsledcích. 5. Chybějící runtime označit explicit skip/neověřeno, ne green pass.

**Ověření:** dostupný podporovaný runtime dokončí všechny scénáře, neověřené konfigurace nepředstírají success. Bez výsledků nelze bezpečnost/runtime/image uzavřít na 9,5. SEC-VERIFY je stejná důkazní mezera.

### QA-03 — P1: CI publikuje web bez kontroly Rustu, balíčku a cest uživatele

**Důkaz:** jediný tracked workflow `.github/workflows/pages.yml` pouze checkout/setup/upload/deploy; nenajdeme Cargo tests/build ani installer, completions či web accessibility checks.

**Doporučení a postup:** 1. Přidat povinný Linux build/test/package job pro změny runtime/installer/image. 2. Automatizovat čistý shell completion setup a minimální docs copy/paste probes. 3. Web kontrolovat keyboard semantics, clipboard failure a responsive rozměry. 4. Živé containment testy oddělit od fast mock sady a publish vázat na relevantní úspěšné kontroly. 5. Nevyžadovat drahé upstream login testy na každém textovém commitu; zavést release gate.

**Ověření:** cíleně vadná fixture nebo regrese relevantního kontraktu rozbije příslušný check před release/deploy; chybějící Podman se hlásí jako chybějící release evidence. UI estetika/mobile zůstávají neověřené, dokud není skutečný render.

### QA-04 — P2: chybí explicitní a ověřená kompatibilitní matice

**Důkaz:** README `35–39`, `Cargo.toml:1–9` (bez rust-version), Dockerfile `19–24`, runtime flags `src/lib.rs:1507–1527`, Pages workflow. Jediný ověřený build zde je Rust1.99/x86_64/Ubuntu24.04 WSL bez Podman; z něj nelze odvozovat minimum Rust, ARM, SELinux ani aktuální upstream auth kompatibilitu.

**Doporučení a postup:** 1. Zvolit úzké minimum/tested Rust a konkrétní Linux/architekturu/Podman/OCI/cgroups kombinaci. 2. Sladit s runtime preflight RT-07 a verzovanými agent vstupy DIST-05. 3. Minimum Rust ověřit skutečným build/test a teprve potom deklarovat rust-version. 4. Další architektury/SELinux označit supported až po smoke. 5. S každým upgrade opakovat relevantní kombinace, ne přenášet green status z jiné konfigurace.

**Ověření:** každá supported položka má dohledatelný build/test/runtime důkaz včetně agent startu; neověřená kombinace je jako taková označená. Nevyžaduje se podpora všech Linux distribucí. Širší QA subagentovy nálezy CLI/login integrace jsou deduplikovány do QA-01/02; CI do QA-03.

## Konkrétní pořadí oprav a uzavírání

1. **Zabránit ztrátě/obnovení stavu a zprovoznit vstup:** RT-01/02, RT-05/06, DIST-01/03; opravy doplnit okamžitě regresními fixture tests. DOC-01 opravit současně, aby uživatel nerozhodoval podle nepodloženého příslibu.
2. **Nezávisle uzavřít hranice izolace:** SEC-N1/N2 a QA-02 na autorizovaném bezpečném prostředí. Bez výsledků žádné prohlášení o containment bezpečnosti. Při potvrzení vady opravit konkrétní IO/mount kontrakt a opakovat sentinel/regresní scénáře.
3. **Procesní a instalační kontrakty:** RT-03/04/07, UX-03/04, DIST-02/04/05; následně end-to-end source/package/init journey na čistém Linuxu.
4. **CLI, návody a web:** UX-01/02, DOC-02/03, WEB-01..04, WEB-N1. Skutečná browser klávesnice, screen reader, mobile/zoom a copy permission failure.
5. **Trvalá ochrana:** QA-01..04 a lehké parity AR-02/03; žádné rozšiřování zakázaného produktového rozsahu. Po každé logické opravě nejdřív cílená kontrola, pak relevantní regresní sada.

## Protokol nového ověření po opravách

Toto kolo neobsahuje opravy, takže druhé hodnocení by teď nemělo co potvrdit. **Opravující revize: žádná. Nové skóre po opravách: žádné.**

Po dodání oprav:

1. Zafixovat nové SHA, čistotu pracovní kopie/explicitní patch a nezměněný rozsah i kritéria. Zachovat tuto recenzi jako baseline; nový výsledek zapsat odděleně do dalšího kola v tomto souboru.
2. Spustit čerstvé kritiky s novým kontextem. Dostanou pouze nový source, AGENTS.md/produktový kontrakt, pevná kritéria a neutrální acceptance scénáře. **Nedostanou stará skóre, staré hodnoticí odstavce, tento report jako argument ani autorovu obhajobu.** V jejich scope explicitně vyřadit score-bearing review artefakty.
3. Rozdělit všech osm oblastí a bezpečnost/reálnou integraci kontrolovat nezávisle. Implementátor není jediný hodnotitel své změny. Noví kritici musí vedle acceptance hledat i další relevantní slabiny.
4. Reprodukce zopakovat na novém SHA; doložit cílené výsledky, regresní scénáře, skutečnou Podman/agent/browser matrix a každou mezeru. Mock, statickou kontrolu a živý test rozlišovat i při úspěchu.
5. Teprve po odevzdání nezávislých známek porovnat baseline. Neshodu řešit konkrétním důkazem/reprodukcí, ne upravením váhy nebo zamlčením neověřené oblasti.
6. Pokud některá oblast nemá 9,5, existuje otevřený P0/P1 nebo zásadní neověřená cesta, zapsat **cíl NEPOTVRZEN**, další postup a podmínky. Potvrdit až při splnění všech osmi oblastí; nikdy nenavyšovat známku jen kvůli dosažení prahu.

**Aktuální rozhodnutí: cíl NEPOTVRZEN.** Zelený unit build je užitečný důkaz kompilace/helperů, ale nevyvrací doložené vady ani chybějící integrace. Není žádný potvrzený post-fix výsledek.

## Nezávislé kolo po opravách R1 — 4. 10. 2026

Výše uvedená baseline a kritéria jsou zachována beze změny. Toto hodnocení platí výhradně pro immutable Git tree **`0dd0da75801b7c82a05b4db920ab4d7b78e01c4a`**, vytvořený dočasným indexem z opravených zdrojů; skutečný index ani HEAD nebyly změněny. Zdroj lze obnovit pomocí `git archive <tree>`. Review artefakt nebyl součástí snapshotu. Následující opravy v pracovním stromu těmito známkami hodnoceny nejsou.

Tři čerství kritici s odděleným kontextem dostali zdroj, projektový kontrakt, původní pevná K1–K5 a neutrální acceptance scénáře. Nečetli baseline skóre, ostatní kritiky ani autorovu obhajobu a zdroj needitovali. Jejich odevzdané známky jsou převzaty doslova; nebyly přepočítány ani zvýšeny:

| Oblast | K1 | K2 | K3 | K4 | K5 | Celkem | Nezávislý kritik |
|---|---:|---:|---:|---:|---:|---:|---|
| Funkční správnost CLI/runtime | 2 | 1 | 1 | 1 | 2 | **7/10** | review_runtime_r1 |
| Architektura a udržovatelnost | 2 | 1 | 1 | 1 | 1 | **6/10** | review_runtime_r1 |
| Bezpečnost a izolace | 2 | 1 | 2 | 1 | 1 | **7/10** | review_qa_isolation_r1 |
| Terminálové UI/UX | 2 | 1 | 1 | 2 | 1 | **7/10** | review_runtime_r1 |
| Veřejná dokumentace | 2 | 1 | 1,5 | 1,5 | 2 | **8/10** | review_web_dist_r1 |
| Web UI/UX a přístupnost | 1,5 | 1 | 1,5 | 1,5 | 1,5 | **7/10** | review_web_dist_r1 |
| Instalace, distribuce a obraz | 2 | 1,5 | 1,5 | 1,5 | 1,5 | **8/10** | review_web_dist_r1 |
| Testy, CI a regresní ochrana | 2 | 1 | 2 | 1 | 2 | **8/10** | review_qa_isolation_r1 |

### Nově potvrzené nálezy R1

- **R1-01 — P1:** `env rm` a `login rm` kontrolují leaf, ale následují symlink rodiče `envs`/`logins`. Nezávislý kritik v disposable fixture odstranil vlastní sentinel mimo deklarovaný state subtree; oba příkazy skončily 0. Opravit ukotvení celého rodičovského stromu a rekurzivního odstranění, přidat negativní regresi.
- **R1-02 — P1:** skutečný embedded login Bash wrapper kontroluje pouze leaf. Symlink `HOME/.codex` přenesl outside fixture do saved loginu; dangling parent původní login odstranil. Kritik provedl celý `ac` s fixture runtime metadata, ale skutečným shell wrapperem a Rust finalizací; oba případy exit 0 bez diagnostiky. Opravit FD ukotvení čtení i zápisu v kontejneru a neinterpretovat neplatný rodič jako logout.
- **R1-03 — P2:** fake Podman mění staging přímo a embedded wrapper nespouští, takže současná zelená sada R1-02 nezachytí. Přidat samostatnou skutečnou regresi wrapperu a úplného copy-in/agent/copy-out/finalize kontraktu.
- **R1-04 — P2:** pozdější `--` přepíše již přijaté poziční argumenty: `ac codex --no-login exec -- -` předal `[codex, -]`, exit 0. Zachovat předchozí argumenty a ověřit skutečné předané argv.
- **R1-05 — P2:** vlastní JSON parser přijímá `\u+061`; syntetické neplatné `.claude.json` se zobrazí jako uložený login a uloží při finalizaci, exit 0. Ověřit přesně čtyři ASCII hex číslice a negativní list/finalize regresi.
- **R1-06 — P3:** Bash/Zsh nabízejí `--` pro `shell`, který jej odmítá. Bash/Fish chybně považují hodnotu `--env --` za passthrough delimiter. Dokončování musí respektovat parserovou gramatiku a spotřebování hodnot voleb.
- **R1-07 — P3:** sedm samostatných `podman info` dotazů násobí startup latency bez průběžné diagnostiky. Kritik naměřil jediný info dotaz 8,23 s a připravené init 23,45 s; první smoke timeout 60 s. Sloučit údaje do jednoho typově validovaného host snapshotu a měřit skutečný startup.
- **R1-08 — P2:** kopírovaný rychlý start po selhání `cd` spustí init i agenta v původním adresáři a skončí 0. Opravit podmínění sekvence ve webu a README a provést skutečný negativní shell journey.
- **R1-09 — P2:** web Cargo panel instaluje do Cargo bin, ale společný PATH přidává pouze local bin. Kritik skutečně sestavil a nainstaloval Cargo variantu; `command -v ac` s webovým PATH selhal. Sladit PATH všech variant, včetně custom CARGO_HOME, a ověřit kopírované bloky.
- **R1-10 — P2:** package archivu uvnitř cizího Git repozitáře připisuje RELEASE jeho commit a `clean`, přestože commit obsahuje pouze nesouvisející fixture. Ověřit odpovídající Git top-level a zahrnout untracked zdroj do stavu; neznámou provenance pravdivě označit `unknown`.
- **R1-11 — P3:** CLI help absolutně tvrdí jediný mount a nepřipojený host home, ačkoli default mountuje auth staging a zvoleným projektem může být home. Sladit s přesnějšími veřejnými návody.

### Skutečné výsledky a omezení R1

- Nezávisle **PASS** Rust **1.85.0 i 1.99.0**, locked test/build, **20/20** unit testů; runtime **18/18**, skutečné Bash/Zsh/Fish **3/3**, distribuce **9/9**, OpenCode SQLite adapter **8/8**. Source archive, release package/checksum/extrakce/instalace s mezerami a vlastní externí Cargo konfigurací prošly. Corrupted skutečný packaged binary i tarball byly odmítnuty exit 1. Rootova plná stabilní source/archive kontrola stejného tree také skončila 0.
- Nezávislý QA/isolation kritik dokončil plný live smoke **exit 0** na WSL Ubuntu 24.04, rootless Podman **4.9.3**, crun **1.14.1**, systemd cgroups v2. Doloženy sentinely host home/skrytá `.agentcage`, UID/GID, 0700/0600, RO root, writable projekt, disposable home, přesné streamy, default **2 CPU / 4 GiB / 256 PID**, CPU throttling, PID EAGAIN a memory OOM137. Skutečné help/version starty všech čtyř připnutých agentů prošly. Skutečný syntetický OpenCode account import/update/export/Console logout prošel bez host SQLite/session historie. Další live fixture doložil persistent `--env`, konflikt s `--no-login` a prázdný OpenCode účet při no-login bez změny host saved-login bytes. Vlastní kontejnery po testech: 0.
- Runtime kritik nezávisle doložil produkční login wrapper při SIGINT **130**, SIGTERM **143** a controlling-PTY Ctrl-C **130**, včetně synteticky aktualizovaného loginu, odstraněného stagingu a teardownu. První kompletní smoke skončil timeoutem; konečný souhrn jeho druhé relace po zániku procesu je **UNKNOWN**, nikoli PASS. To neruší zvlášť zachycené výsledky ani samostatný dokončený QA smoke.
- Skutečný Chromium **151.0.7922.34**: keyboard tabs, mobile 320/360/390, touch copy, fokus, denied/unavailable clipboard recovery i následné ruční Ctrl+C a čtení schránky prošly. Nezávislé skutečné `chrome.tabs.setZoom` **100/200/400 %** zachovalo CSS zoom=1, outerWidth=1280; innerWidth **1264/632/316**, DPR **1/2/4**, bez overflow ve všech panelech. AX tree má jeden H1 a správné tab/panel stavy; naměřený minimum textového kontrastu **8,21 : 1** není úplný WCAG audit. Dodaný R1 browser test zatím používá CSS zoom.
- **PASS** checksumem ověřený actionlint **1.7.7** a schema/dependency kontrola workflow; sh syntax entry points. Bez opt-in smoke skutečně hlásí **77 / UNVERIFIED**, s required **1 / UNVERIFIED**. Skutečné GitHub-hosted běhy a deployment neproběhly.
- Distribuční autor mimo nezávislé review provedl dva čerstvé no-cache image buildy; připnuté dpkg verze, native binární SHA a labels se shodovaly, image ID se lišila. Nezávislý distribuční kritik používal připravený izolovaný image, takže jeho vlastní fresh network build je **UNKNOWN**. Záznam nelze zaměnit za bitově reprodukovatelný image.
- Baseline binárka pod runtime regresí měla **13 testů / 31 assertion failures / 0 harness errors**; jde o dříve provedené root/QA srovnání, nikoli nezávislý R1 kritikův vlastní failing-before výsledek. Nové R1 nálezy vyžadují vlastní reprodukci před/po opravě.
- **Stále UNKNOWN:** dedicated-provider OAuth a refresh všech agentů, skutečný screenreader, přirozený browser permission dialog, síťový bootstrap proti mutable main, aktuální veřejný deployment a hosted CI. SELinux a další runtime/architektury nejsou doložené podporované kombinace. Syntetický auth roundtrip ani help/version není OAuth důkaz.
- Samostatný post-patch credential candidate reviewer workflow skillu `codex-security:fix-finding` byl službou zastaven pro možný cybersecurity risk; statická fallback žádost byla také odmítnuta. Jeho nedokončenou kontrolu nelze označit PASS ani nahrazovat vlastním přesvědčením. Nezávislé běžné R1 contract review výše zůstává samostatným důkazem.

**Rozhodnutí R1: cíl NEPOTVRZEN.** Žádná oblast nedosáhla 9,5; dvě nové P1 vyžadují opravu. Další kolo musí posoudit nový immutable tree se stejným rozsahem a kritérii, zopakovat nově selhávající regrese, doložit aktuální live/browser kontroly a zachovat otevřené důkazní mezery.

## Nezávislé kolo po opravách R2 — 4. 10. 2026

Baseline, rozsah a K1–K5 zůstávají beze změny. Toto hodnocení platí pouze pro immutable Git tree **`028758bfc7b73a7eeb29e72f5cc5c60b76dfa95f`**, vytvořený dočasným indexem, bez změny skutečného indexu či HEAD. Review artefakt nebyl ve snapshotu. Novější pracovní opravy jsou předmětem dalšího kola, nikoli těchto známek.

Dva čerství kritici s odděleným kontextem posoudili všech osm oblastí. Dostali frozen zdroj, AGENTS.md, původní pevná kritéria a neutrální acceptance; nečetli staré známky ani autorovu obhajobu. Runtime kritik měl před konečným hodnocením rozšířený scope o bezpečnost/izolaci a QA/CI. Odevzdané známky jsou převzaty doslova:

| Oblast | K1 | K2 | K3 | K4 | K5 | Celkem | Nezávislý kritik |
|---|---:|---:|---:|---:|---:|---:|---|
| Funkční správnost CLI/runtime | 2 | 1 | 1,5 | 1,5 | 1,5 | **7,5/10** | review_runtime_r2 |
| Architektura a udržovatelnost | 2 | 1 | 1,5 | 1,5 | 1,5 | **7,5/10** | review_runtime_r2 |
| Bezpečnost a izolace | 2 | 1 | 2 | 1,5 | 1,5 | **8/10** | review_runtime_r2 |
| Terminálové UI/UX | 2 | 1,5 | 1,5 | 2 | 1,5 | **8,5/10** | review_runtime_r2 |
| Veřejná dokumentace | 2 | 1 | 1,5 | 2 | 1,5 | **8/10** | review_web_dist_r2 |
| Web UI/UX a přístupnost | 2 | 1,5 | 2 | 2 | 2 | **9,5/10** | review_web_dist_r2 |
| Instalace, distribuce a obraz | 2 | 1,5 | 2 | 2 | 2 | **9,5/10** | review_web_dist_r2 |
| Testy, CI a regresní ochrana | 2 | 1 | 2 | 1 | 1,5 | **7,5/10** | review_runtime_r2 |

### Potvrzené nové nálezy R2

- **R2-01 — P1:** Claude `.claude.json` s `primaryApiKey` typu boolean, number nebo array byl při produkčním wrapper/copy/finalize kontraktu vyhodnocen jako logout. Kritik doložil odstranění původního syntetického host loginu, exit 0 a žádnou diagnostiku ve všech třech variantách. Metadata Podmanu byla fixture; wrapper a finalizace byly skutečné. Neplatná struktura credential musí být chyba, nikoli nepřítomnost loginu. Relevantní frozen řádky `src/lib.rs:1330–1378`, `1640–1650`.
- **R2-02 — P2:** skutečný Fish při `ac codex -` nenabízel passthrough `--`; Bash a nativní Zsh widget jej nabízely. Shellová sada tuto kandidátní volbu nevyžadovala. Doplnit paritu s parserem a negativní i pozitivní skutečnou regresi. Frozen `src/lib.rs:2758–2763`.
- **R2-03 — P2:** neUTF8 argument za `--` způsobil panic z `env::args()`, exit 101, před spuštěním runtime. Linux raw argv i help slibují předání beze změny. Použít nativní OsString pro agent tail a diagnostikovat neplatný wrapper vstup bez panicu. Frozen `src/lib.rs:99`.
- **R2-04 — P1:** pomocné veřejné návody `docs/index.md:33`, `init.md:18`, `SECURITY.md:49` oddělují `cd`, init a agent bez podmínění. Neexistující cílový adresář při skutečném `ac` vedl k vytvoření `.agentcage` a skutečnému kontejneru nad původním adresářem; init postup skončil 0. README, TUTORIAL a HTML ve stejné negativní kontrole správně skončily 2 a adresář neinicializovaly.
- **R2-05 — P2:** clone postup v `docs/index.md:19` po skutečném Git selhání 128 pokračoval existujícím checkoutem, jeho installerem, skutečným release buildem a instalací, celkem exit 0. Podmínit návazné kroky úspěchem Git, včetně všech publikovaných variant.
- **R2-06 — P2:** doslovné `curl … | sh` v README a HTML při skutečně nedostupné HTTPS proxy skončilo 0, přestože curl skončil 7. Nejprve úspěšně stáhnout script a až poté spustit installer; negativně ověřit doslovný kopírovaný příkaz.

### Skutečné výsledky a omezení R2

- Runtime kritik nezávisle **PASS** stable Cargo **23/23**, black-box runtime **24/24**, skutečné Bash/Fish/Zsh **3/3**, produkční login wrapper + Rust finalize **6/6**, FD file transfer **10/10**, OpenCode SQLite bridge **8/8** a actionlint všech čtyř frozen workflows. Kladné výsledky nevylučují jím zvlášť reprodukované typové smazání loginu, Fish delimiter ani neUTF8 panic.
- Jeho kompletní scoped live Podman smoke skončil **PASS**: UID/GID, private tmpfs home, skrytá `.agentcage`, writable projekt, RO root, skutečné CPU/PID/memory enforcement, všechny čtyři agent help/version, syntetický OpenCode import/update/logout, shell SIGINT/SIGTERM, controlling-PTY Ctrl-C, teardown a vlastní cleanup/restart race. Doplňkový live no-login child **0/1/42/125–130 PASS**, live login wrapper **0/1/42/125/126/127 PASS**. Doplňková session 71634 po zániku vracela `Unknown process id`; nevyzvednuté login **128/129/130** a login-enabled **SIGINT/SIGTERM/SIGHUP** jsou **UNKNOWN**, nikoli PASS. Scoped kontrola jeho labelu nenašla zbylý kontejner.
- Distribuční kritik nezávisle provedl Rust/Cargo **1.85.0**, native x86_64 ELF a skutečné script bootstrap, Git/source, Cargo Git a binary archive instalace → publikovaný PATH → projekt s mezerami → skutečný init → login-enabled native Codex help. Git/curl transport byl nasměrován na jeho immutable mirror/HTTP fixture; Git, curl, Cargo, installer, ac, Podman i agent byly skutečné. Síťový mutable main tím není prokázán.
- **PASS** jeho installer/package s konfliktní Cargo target/config a externím target-dir s mezerami; package i binary checksum; odmítnutí poškozeného binary/archivu a cross-target package; odpovídající clean Git provenance i archive `unknown`; bootstrap Git/compiler failure **128/101** bez instalace a s vlastním temp cleanupem.
- **PASS vlastní čerstvý `--no-cache` image build kritika** v samostatném rootless store. Dockerfile SHA-256 **`3b04a60d3524f278464990866d6f59a0fcf39f1b28ed20a1e40b3874723ecd33`**, image **`6ccc8eb7a471882c463b7de4da9649bae74eb64e0b88fd76903d84b2b70319b3`**, user **1000:1000**, odpovídající recipe/base/version labels a všechny síťové hash kontroly. Na jeho vlastním image všech **8** skutečných help/version runů skončilo 0: Codex **0.160.0**, Claude **2.1.289**, OpenCode **1.18.34**, Antigravity **1.2.16**. Skutečné ac shell potvrdilo UID/GID, RO root, ELF všech čtyř binary, Node **22.23.3**, npm **10.9.9**. Nejde o tvrzení bitové reprodukovatelnosti image.
- Jeho skutečný Chromium **153.0.8010.12**: keyboard tabs/focus, clipboard grant, Permissions Policy denial, unavailable fallback a ruční Ctrl+C jednořádkových i víceřádkových příkazů **PASS**. Všechny panely při 320/360/390 a skutečném browser zoom 100/200/400 % (viewport **1264/632/316**) bez overflow; mobile touch/copy a minimálně 44px ovládání; skip-link fokus na main, forced-colors focus, JS-disabled manuální text, správné AX tab/panel/landmark/button/live semantics **PASS**. Minimum kontrastu 115 viditelných textových uzlů **7,3885:1**; nejedná se o úplný WCAG audit.
- Rootova samostatná úplná stable source/archive kontrola stejného R2 tree skončila **0**, včetně release build/package/checksum/extrakce/no-build install. Tento výsledek je autorova kontrola, není další nezávislá známka.
- **UNKNOWN:** skutečný screen reader, dedicated-provider OAuth/refresh/autentizovaná coding session a provider revocation; hosted GitHub install endpoint, CI/release/Pages publikace; další neslíbené runtime/arch/SELinux kombinace. Runtime kritik svůj vlastní minimum-Rust a baseline before/after běh neprovedl, přestože jiné oddělené kontroly existují. Nedokončený skill candidate review z R1 zůstává neuzavřený; běžné contract review jej nenahrazuje.

**Rozhodnutí R2: cíl NEPOTVRZEN.** Web a distribuce dosáhly 9,5, ostatních šest oblastí nikoli; dvě P1 a zásadní auth důkazní mezera brání uzavření. Další kolo musí hodnotit nový frozen source, zopakovat typové/argv/shell/docs regrese a skutečné integrace se stejnými kritérii. Předběžné autorovy opravy nejsou důkazem splnění prahu.

## Nezávislé kolo po opravách R3 — 4. 10. 2026

Původní rozsah, baseline a K1–K5 zůstávají beze změny. Hodnocený immutable Git tree je **`f0045dc8c1d42bceaf3a636a8e834fd62213c47d`**; skutečný index a HEAD nebyly změněny. Review soubor není součástí snapshotu. Novější pracovní opravy budou vyžadovat nové nezávislé kolo.

Tři čerství kritici dostali pouze frozen zdroj, projektová pravidla, původní pevná kritéria a neutrální acceptance. Nečetli stará skóre ani autorovy PASS závěry. Všech osm odevzdaných známek je převzato doslova.

| Oblast | K1 | K2 | K3 | K4 | K5 | Celkem | Nezávislý kritik |
|---|---:|---:|---:|---:|---:|---:|---|
| Funkční správnost CLI/runtime | 2 | 1 | 1,5 | 1,5 | 1,5 | **7,5/10** | review_r3_planning |
| Architektura a udržovatelnost | 2 | 1,5 | 1,5 | 1,5 | 1,5 | **8/10** | review_r3_planning |
| Bezpečnost a izolace | 2 | 1 | 2 | 2 | 1 | **8/10** | review_qa_isolation_r3 |
| Terminálové UI/UX | 2 | 1,5 | 1,5 | 2 | 1,5 | **8,5/10** | review_r3_planning |
| Veřejná dokumentace | 2 | 1,5 | 2 | 2 | 2 | **9,5/10** | review_web_dist_r3 |
| Web UI/UX a přístupnost | 2 | 1 | 2 | 2 | 1,5 | **8,5/10** | review_web_dist_r3 |
| Instalace, distribuce a obraz | 2 | 1,5 | 2 | 2 | 2 | **9,5/10** | review_web_dist_r3 |
| Testy, CI a regresní ochrana | 2 | 2 | 2 | 1 | 2 | **9/10** | review_qa_isolation_r3 |

### Potvrzené nové nálezy R3

- **R3-01 — P1:** Codex přijímá libovolné bajty a Claude OAuth libovolný neprázdný objekt. Runtime kritik přes skutečný Podman a embedded wrapper doložil nahrazení platného uloženého loginu poškozeným JSON, `false`, prázdným Codex souborem, `tokens:42` a chybně typovanými Claude access/refresh/expiry údaji, s exit 0. Samostatný QA kritik reprodukoval i OAuth objekt obsahující pouze nesouvisející nastavení. Kontrolovat skutečné formáty připnutých agentů, rozlišit platný logout a chybu a zachovat celý přijatý snapshot. Frozen `src/lib.rs:1383–1421`, `1748`.
- **R3-02 — P2:** OsString zachová argument pouze do Podmanu; skutečný OCI JSON transport zamění `ff`/`fe` za UTF-8 replacement bytes `efbfbd`. Runtime kritik s loginem i bez něj předal `opaque-ff-fe`, agent dostal změněný argument a status 0. Je nutná bezpečná byte-preserving cesta přes OCI a skutečná regresní kontrola, nikoli jen kontrola argv mocku.
- **R3-03 — P3:** globální help, completions a login list zbytečně vyžadují platný pracovní adresář. Při odstranění vlastního cwd skutečný `--help` skončil 1 bez helpu. Frozen `src/lib.rs:165`.
- **R3-04 — P3:** zavřený stdout při helpu způsobuje Rust BrokenPipe panic, exit 101. Ověřit běžné Unix pipe ukončení i zachování finalizace agentových session. Frozen `src/lib.rs:2562`.
- **R3-05 — P2:** prázdná skutečná HTTP 200 odpověď se při publikovaném curl postupu v README a HTML spustí jako prázdný shell script. Výsledek 0, žádná instalace, návazné `&&` se provede. Distribuční kritik použil skutečný HTTP server/curl; neúplná odpověď naopak správně selhala 18. Frozen README `63–64`, `70–71`, HTML `130`.
- **R3-06 — P2:** Orca 46.1 + Chrome 151 po browse čtení panelu neoznámí zamítnutí schránky ani návod na obnovu. Web kritik reprodukoval **3/3** deep průchody; po 15 sekundách pouze příkaz a „selected“. AT-SPI obsahuje celý status s live polite/atomic true a skutečné text/children události, basic průchod jej přečte. Jde o efektovou mezeru, nikoli chybějící ARIA data; příčina v readeru versus pořadí selection/status zůstává UNKNOWN. Frozen HTML `139`, `247–251`.
- **R3-07 — P3:** Orca mluví jméno Cargo Git tabu a „2 of 3“, ale nikoli explicitní selected; panel při vstupu jen „scroll pane“, bez názvu. AT-SPI selected/name jsou správné. Je doloženo omezení kompatibility; AX kontrola sama neprokazuje mluvenou identifikaci. Frozen HTML `123–143`, `260–267`.
- **R3-08 — P2:** při skutečném ENOSPC na vlastním 1MiB tmpfs se první platný credential uloží, druhý platný 2MiB credential selže, exit 1 a staging s vrácenými změnami je smazán. QA kritik reprodukoval produkční Rust finalize nad dvěma syntetickými validními UTF-8 AuthDotJson soubory: první NEW, druhý OLD, žádná obnovovací kopie. Atomické rename jednotlivých souborů nezajišťuje celistvost per-agent snapshotu. Frozen `src/lib.rs:1743–1750`, `550`, `1756–1759`.

### Skutečné výsledky a omezení R3

- Runtime kritik nezávisle provedl minimum Rust **1.85.0** i stable **1.99.0**, Cargo **24/24**, runtime **25/25**, wrapper **11/11**, skutečné shells **3/3**, SQLite adapter **8/8**, FD transfer **10/10 PASS**. Další vlastní repro výše přesto odhalila chybné auth a skutečný opaque argv transport.
- Jeho vlastní kompletní live smoke **PASS**: UID/GID/home/root/mounts, skutečné CPU/PID/OOM enforcement, startup čtyř native agentů, syntetický OpenCode account roundtrip/logout, shell signály, PTY, vlastní cleanup/restart race. Doplňkově exact child **0/1/42/125/126/127/130**, binary pipe/stdout/stderr; login-enabled **SIGINT 130/SIGTERM 143/SIGHUP 129/PTY Ctrl-C 130**, obě pořadí concurrent unchanged/update, divergence, logout/update a lokální revocation/new login **PASS**. Skutečné validní argv včetně prázdných, quotes, newline a Unicode **PASS**, neplatné UTF-8 **FAIL**. Bash Readline PTY **14** completion případů PASS. Po kontrole jeho labelu žádné zbylé kontejnery.
- Distribuční kritik nezávisle provedl source, stdin bootstrap uvnitř cizího Cargo projektu, Cargo Git, clone, package/checksum/install/PATH a negativní failure/interrupt cesty se skutečnými Git/curl/Cargo/install/ac nástroji **PASS**. Transport používal vlastní immutable mirror a HTTP fixture, nikoli mutable veřejný main. Minimum Rust **1.85.0**, package LF/0755, provenance a odmítnutí korupce **PASS**. Bootstrap SIGINT **130**/SIGTERM **143** odstranil vlastní staging a neinstaloval binárku.
- Vlastní prázdný Podman store distribučního kritika → skutečný frozen embedded image build **PASS**. Image **`b826fb602de6e6378be63b3f0dd77de5b163ef2db6892863c4d578aeda56b9db`**, recipe SHA-256 **`3b04a60d3524f278464990866d6f59a0fcf39f1b28ed20a1e40b3874723ecd33`**, user **1000:1000**. Síťové checksumy a osm native help/version startů **PASS**: Codex **0.160.0**, Claude **2.1.289**, OpenCode **1.18.34**, Antigravity **1.2.16**. Vlastní shell potvrdil RO root/EROFS 30. Po bězích žádné kontejnery. Není to tvrzení bitové reprodukovatelnosti image.
- Jeho skutečný Chromium **151.0.7922.34**, Playwright **1.62.1**: keyboard tabs/focus, skutečný clipboard grant/denial a unavailable/manual native Ctrl+C, 320/360/390 px všechny panely, 44px targets, skip-link, forced colors a JS disabled **PASS**. Skutečný browser zoom **1/2/4** měl viewport **1264/632/316** bez overflow. Zkontrolované screenshots bez překrytí, minimum paletového textu **5,39:1**, copy border **3,33:1**.
- Živý **Orca 46.1 → SpeechDispatcher 0.12rc2 → eSpeak NG 1.51 → WAV** web kritik provedl jeden basic a tři deep průchody; syntézy exit 0. Basic error feedback PASS, deep feedback **FAIL** a selected/panelName omezení výše. Exit 0 harnessu ani správný AT-SPI strom nejsou blanket PASS screen readeru.
- QA/izolační kritik nezávisle potvrdil obě Rust verze, všechny uvedené frozen Python sady, vlastní plný skutečný containment smoke, actionlint **1.7.7** všech čtyř workflows, exact Git archive/release/package/install/corruption a vlastní skutečný Chromium check. Runtime **25/25**, wrapper **11/11**, shells **3/3** zopakoval i nad skutečně instalovaným MSRV release. Stable binary SHA-256 **`9a362f77f26f7036be940e8e288b3550934275f5ef3d668b43420c10c52e2de1`**, installed MSRV release **`5446d1622a14fb0e9e429de8ab164ef14893f7aa8e36dc7fb46967c66bc32e24`**. Missing-runtime gate skutečně vrací **77**, s required **1**. K4 QA snížil za konkrétní regressions, které všechny stávající gaty přehlédly; K2/K5 izolace za typovou korupci a částečný ENOSPC snapshot bez obnovy.
- Rootovy oddělené úplné minimum/stable source/archive kontroly frozen R3 skončily **0**, jeho live smoke **12/12 skupin PASS**, skutečný Chromium **6/6 skupin PASS**. Tyto autorovy kontroly nejsou další nezávislé hodnocení. R2 before pod novými R3 OpenCode export/provider regresními testy měl **3 metody / 11 assertion failures / 0 harness errors**, after R3 wrapper **11/11 PASS**.
- **UNKNOWN:** vyhrazené provider OAuth/login/refresh/autentizovaná coding session všech čtyř agentů, provider revocation, hosted GitHub CI/release/Pages a mutable main bootstrap; další screen readery/prohlížeče a fyzická touch zařízení. Neslíbené runtime/arch/SELinux konfigurace nejsou ověřené podporované kombinace. Samostatný service-blocked skill candidate review zůstává nedokončený; běžné contract review jej nenahrazuje.

**Rozhodnutí R3: cíl NEPOTVRZEN.** Šest oblastí nesplňuje práh; P1 a zásadní provider-auth mezera jsou otevřené. Opravy ENOSPC, typů credentialů, OCI přenosu a screen reader feedback potřebují nové zmrazení a nezávislé review se stejnými kritérii. Autorův odhad není nezávislým hodnocením.

## Nezávislé kolo po opravách R4 — 4.–5. 10. 2026

Původní rozsah, baseline a K1–K5 jsou zachovány. Immutable Git tree **`c41879fa2027f86cab418235d2a75ffb4eefbaaa`** byl vytvořen dočasným indexem; skutečný index a HEAD zůstaly beze změny. SHA-256 přesného archivu je **`7e6e2cc0f23eb06fe64db46142641fa7f016b7e2f89519caff2a3bf1833de5be`**. Review soubor není součástí snapshotu. Kritici na konci ověřili shodu 40 běžných source souborů; osobní auth odkazy byly z hodnocení vyřazeny a nebyly čteny.

Tři čerství kritici dostali pouze tento frozen source, AGENTS.md, původní pevná kritéria a neutrální acceptance. Nečetli předchozí známky ani autorovy PASS závěry. Následující dvě odevzdaná hodnocení jsou převzata doslova; chybějící třetí hodnocení není dopočítáno z průběžných výsledků.

| Oblast | K1 | K2 | K3 | K4 | K5 | Celkem | Nezávislý kritik |
|---|---:|---:|---:|---:|---:|---:|---|
| Funkční správnost CLI/runtime | — | — | — | — | — | **NEDODÁNO** | review_runtime_r4; service-blocked final |
| Architektura a udržovatelnost | — | — | — | — | — | **NEDODÁNO** | review_runtime_r4; service-blocked final |
| Bezpečnost a izolace | 2 | 2 | 2 | 2 | 2 | **10/10** | review_qa_isolation_r4 |
| Terminálové UI/UX | — | — | — | — | — | **NEDODÁNO** | review_runtime_r4; service-blocked final |
| Veřejná dokumentace | 2 | 2 | 2 | 2 | 2 | **10/10** | review_web_dist_r4 |
| Web UI/UX a přístupnost | 2 | 2 | 2 | 2 | 2 | **10/10** | review_web_dist_r4 |
| Instalace, distribuce a obraz | 2 | 2 | 2 | 2 | 2 | **10/10** | review_web_dist_r4 |
| Testy, CI a regresní ochrana | 2 | 2 | 2 | 2 | 2 | **10/10** | review_qa_isolation_r4 |

### Potvrzený nový nález R4

- **R4-01 — P2:** celý host snapshot může být změněn po neúspěšném kontejnerovém copy-out. Runtime kritik se skutečným Podman/OCI a výhradně vlastními syntetickými auth soubory odstranil oba credentialy v disposable HOME a nastavil staging `.codex` na `0500`. První exportovaný `unlink(auth.json)` uspěl, druhý skutečně selhal s kernel `EACCES` 13. Wrapper vrátil **1** při child 0 a zachoval **42** při child 42, ale Rust finalize přijal částečný staging jako platný logout: host `auth.json` zmizel, `.codex/auth.json` zůstal OLD. Frozen `src/login_sync.py:137`, `src/lib.rs:89`, `2160`, `2532`. Root i samostatný QA worker následně potvrdili před/po účinek na vlastních fixtures. Doporučení: Rust smí přijmout staging pouze po explicitně dokončeném celém exportu, se zachováním přesného child statusu; přidat skutečnou IO regresi. Novější pracovní oprava není součástí známek R4.

QA/isolation kritik ve svém samostatném přiděleném rozsahu tento nález neobjevil. Jeho doslovné známky zůstávají zachovány; potvrzená ztráta dat a chybějící runtime hodnocení přesto brání uzavření celkového cíle.

### Skutečné výsledky a omezení R4

- QA/isolation kritik: stable Rust **1.99.0**, fast suite **112/112** (25 Rust, 29 runtime, 15 wrapper, 3 shells, 10 distribution, 8 OpenCode, 10 login sync, 12 web commands). Minimum Rust **1.85.0**, celý exact-archive source gate **122/122**, locked release/package/checksum/extract/install/help **PASS**. Vlastní stable release SHA-256 **`6a8f94a9547a0c898191464aa0a5ed66310c30a1b77fe47bcfd3ac5e88a50a7b`**; skutečný installed MSRV archive release **`7b3c6ff01f54eb547590ef43c296bbca4c5d4deb8afaf172714275c38fa89520`**.
- Jeho vlastní required Podman smoke **16/16**, exit 0: UID/GID 1000, 0700 home, RO root/EROFS, writable project, hidden state a host-home sentinel, default 2 CPU/4 GiB/256 PID a skutečné throttle/OOM137/EAGAIN, osm native help/version startů, skutečný OCI opaque argv včetně 70 KiB a binary stdin s login/no-login 0/42, odmítnutí typově poškozeného Codex/Claude celého snapshotu, nativní lokální syntetický Codex API writer/status, OpenCode lokální SQLite account roundtrip/logout, signály/PTY/teardown, stopped-only clean a skutečná restart race. Samostatný kernel ENOSPC **2/2** se čtyřmi child 0/42 scénáři prokázal celý OLD store, private retained recovery, aged cleanup protection a nový save po uvolnění místa. Dalších **8/8** native verzí normal-login/`--env` a tři persistent/disposable shell sessions prošly; skutečné capability sady byly nulové, NoNewPrivs=1, Seccomp=2 a vlastní ambient API/AWS/proxy fixture nebyla předána.
- QA kritik také provedl Chromium **7/7**, skutečně odmítnutou bad-heading fixture a osm negativních gate scénářů včetně child failure 17 a unverified **77** / required **1**. Rootovy samostatné minimum/stable exact-archive source gaty také skončily **0**, jeho required live smoke **16/16** se SHA-256 **`3fcf9b2ca82b33602b26c6bc1991be3c24ca6c732b588f7166bb08363a4cdb56`**. Autorovy kontroly nejsou nezávislým skóre.
- Distribuční/web kritik: všechny **12** published-command a **10** distribution testů PASS; skutečné source, Git, Cargo Git, curl→stdin a literal README/TUTORIAL package journeys přes vlastní immutable mirror a skutečný HTTP server PASS. Reálné HTTP 404, truncated transfer, empty HTTP 200, Git/rustc failure a SIGINT/SIGTERM/SIGHUP **130/143/129** zachovaly předchozí instalaci/sentinel a odstranily vlastní staging. Checksum/corruption, LF/0755, prefix/relative paths s mezerami, cizí Cargo/Git a pravdivá provenance i odmítnutí publikace se zachováním předchozího archive/checksum páru PASS.
- Jeho actual Chromium **151.0.7922.34**: keyboard/skip/focus/roving tabs, clipboard grant/denial/unavailable/manual Ctrl+C/recovery, JS off, 320/360/390 px a skutečné `chrome.tabs.setZoom` 1/2/4 s viewportem **1264/632/316** PASS. Prohlédnuté screenshots bez překrytí; konzervativní textový kontrast **7,39:1**, Copy border **3,33:1**, focus **10,32:1**. Actual Orca **46.1**, AT-SPI2 **2.52.0**, Chrome 151, eSpeak/WAV: **61** syntetizovaných utterances, žádná synthesis failure, nenulové RIFF/PCM vzorky. Ve deep browse průchodu zazněl heading, změna aktivní metody, WhereAmI `2 of 3`, konkrétní Cargo příkaz a celý denied-copy postup s Ctrl+C. Výslovné slovo „selected“ ani první název panelu nebyly tvrzeny jako vyslovené; přístup k aktivnímu příkazu byl doložen skutečnou řečí.
- Jeho fresh rootless image build přesného Dockerfile **--no-cache PASS**, pouze base digest předem dostupný: image **`aeade316ba97f868b2540a2c0ebcdd4aefed35d0b9272ab95210569b6d0f074e`**, digest **`sha256:10b8e6217e92d4d2e554df8f3ad1b4d575142be5398a81449fa5bb32dea92107`**, recipe **`3b04a60d3524f278464990866d6f59a0fcf39f1b28ed20a1e40b3874723ecd33`**. Checksumy všech připnutých zdrojů a skutečné ac/Podman help/version všech čtyř agentů, Node 22.23.3/npm 10.9.9 a UID 1000/home PASS. Nejde o důkaz bitové reprodukovatelnosti image.
- Runtime kritik před zablokováním odevzdal ověřitelné průběžné výsledky: vlastní minimum/stable 25 unit, celý live smoke a kernel ENOSPC PASS, plus skutečné R4-01 repro výše. Jeho závěrečný turn však služba odmítla s důvodem **possible cybersecurity risk**; žádná závěrečná známka doručena nebyla. Převzetí těchto dílčích důkazů nenahrazuje chybějící nezávislé hodnocení. Starší chráněná Codex Security candidate brána je také nadále service-blocked; žádná nebyla prohlášena za PASS ani znovu spuštěna.
- **UNKNOWN:** vyhrazené provider OAuth/login/refresh/autentizovaná coding session všech čtyř agentů a provider revocation, hosted CI/release/Pages a mutable upstream main. Startup, syntetické credentials a lokální writer/status tyto cesty nedokazují. Další screen readery/prohlížeče/fyzická touch zařízení a neslíbené platformy nebyly ověřeny.

**Rozhodnutí R4: cíl NEPOTVRZEN.** Pět doslovně odevzdaných hodnocení dosáhlo 10/10, tři chybí a skutečně potvrzený P2 exportu i zásadní provider-auth důkazní mezera zůstávají otevřené. Další opravený source musí mít nové zmrazení a čerstvé nezávislé hodnocení původního rozsahu a kritérií. Žádné chybějící skóre nebude nahrazeno autorovým přesvědčením.

## Nezávislé kolo po opravách R5 — 5. 10. 2026

Rozsah, baseline a K1–K5 jsou beze změny. Immutable Git tree **`a65301ef47b61402173208968ca3e66d45a5e22f`**, archive SHA-256 **`1717345127875884b53ab854d146d5c295a7e77b1bb18f06d300549868caac1c`**. Skutečný index a HEAD zůstaly beze změny; review soubor není součástí tree. Všichni tři čerství kritici na závěr nezávisle ověřili všech 40 povolených regular blobs proti Git tree/archivu; osobní auth odkazy nečetli. Neznali starší známky, jiné review ani autorovy PASS závěry. Jejich následující kompletní hodnocení je převzato doslova a platí jen pro tento tree, nikoli pro novější opravy.

| Oblast | K1 | K2 | K3 | K4 | K5 | Celkem | Nezávislý kritik |
|---|---:|---:|---:|---:|---:|---:|---|
| Funkční správnost CLI/runtime | 1 | 1,5 | 1,5 | 1 | 1,5 | **6,5/10** | review_runtime_r5 |
| Architektura a udržovatelnost | 2 | 1,5 | 1,5 | 1,5 | 1,5 | **8/10** | review_runtime_r5 |
| Bezpečnost a dokumentovaná izolace | 2 | 2 | 2 | 1 | 2 | **9/10** | review_contract_qa_r5 |
| Terminálové UI/UX | 1,5 | 1,5 | 1,5 | 2 | 1,5 | **8/10** | review_runtime_r5 |
| Veřejná dokumentace | 2 | 2 | 2 | 2 | 2 | **10/10** | review_web_dist_r5 |
| Web UI/UX a přístupnost | 2 | 2 | 2 | 2 | 1 | **9/10** | review_web_dist_r5 |
| Instalace, distribuce a obraz | 2 | 1 | 2 | 1 | 2 | **8/10** | review_web_dist_r5 |
| Testy, CI a regresní ochrana | 2 | 2 | 2 | 1 | 2 | **9/10** | review_contract_qa_r5 |

### Potvrzené nové nálezy R5

- **R5-01 — P2:** host rozpoznává `account.json` s `{"accounts":[{}],"active_account_id":null,"active_org_id":null}` jako saved OpenCode login: `login list` exit 0, vypíše opencode. Skutečný embedded SQLite bridge tentýž dokument odmítne; skutečné OCI `ac opencode -- --help` skončí 1 před agentem, store zůstane zachován. Frozen `src/lib.rs:1420` versus `src/opencode_account.py:32`. Host validation musí souhlasit s přesnými account fields, typy, unikátními IDs a selection vlastního adapteru; nekompletní/invalid dokument není logout.
- **R5-02 — P2:** dokumentace slibuje obnovovací kopii při selhání uložení validovaného snapshotu, ale konflikt skončí před retention. Dvě skutečné souběžné OCI sessions vrátily různé validní syntetické API credentials: první 0, druhá 1/conflict, vítězný store správně zachován; oba stagingy však zmizely a žádná recovery cesta nebyla uvedena. Frozen `LOGIN_PERSISTENCE.md:30`, `src/lib.rs:2186`. Zpřesnit příslib nebo zachovat validní konfliktní kopii bez automatického obnovení. Aktuální pracovní oprava volí zachování kopie.
- **R5-03 — P2:** native `install` píše přímo do existujícího `ac`. Skutečný `ulimit -f 1` ve vlastním package fixture skončil **153**, ale zkrátil původní funkční ELF **884808 → 512 B**; jeho následné `--help` selhalo EACCES. Frozen `install.sh:120`. Připravit kompletní soubor na stejném destination filesystemu a až potom atomicky zveřejnit; ověřit skutečné pozdní IO selhání i přerušení.
- **R5-04 — P3:** při úspěšném native Clipboard kopírování Orca nevysloví status `Copied Cargo Git installation command.`. Dva skutečné deep/keyboard grant průchody mají změněný DOM/AT-SPI a ve druhém SHOWING=true, ale po 15 s žádný odpovídající spoken/WAV výstup. Kontrolní HTML změnilo pouze success `aria-atomic=true → false`; stejný flow poté vyslovil potvrzení, WAV **106430 B / 53193 frames / synthesis exit 0**. Denial alert na originálu funguje. Frozen `docs/index.html:139`, ostatních pět shodných statusů a update `246`. Jde o malé chybějící zvukové potvrzení, nikoli nefunkční kopírování; oprava musí být ověřena na skutečném produkčním HTML.

### Skutečné výsledky a omezení R5

- Runtime kritik doložil vlastní běžný actual OCI harness: empty/Unicode/opaque argv, **71680 B** argument, **71687 B** binary stdin se shodným stdout SHA-256 **`65dbf1148a1b50131cf217ca144f31cb30931934118dfbd70270085d43408f8a`**, streamy/status 0/42, signály INT130/TERM143/HUP129, login-enabled signal save, PTY všech streamů versus pouze stdin/stdout, shell exit42. Syntetický login import/update/logout/no-login, persistent home/rm, unchanged/refresh/conflict/local-rm/freshsave a nativní OpenCode SQLite selection roundtrip PASS. Native osm help/version startů PASS. Cílené Rust **20**, runtime **23**, shells **3**, wrapper **12**, SQLite **7** PASS; locked offline minimum Rust1.85 build PASS. Skutečné EACCES i vlastní 1MiB namespace ENOSPC/recovery pro child0/42 PASS; namespace používá procesní fixture, nikoli OCI/provider. Stable debug SHA-256 **`3546c97e33741bf620bcd839e5ea047bb838368df9954a3a87747471293b9ca7`**. K1/K4 runtime snížil za nedoložený hlavní authenticated-coding workflow; ostatní srážky za výše uvedenou validaci/recovery.
- QA/contract kritik: minimum i stable Rust **25/25** a debug build PASS; samostatné runtime **30**, wrapper **16**, shells **3**, distribution **10**, account **8**, sync **12**, web commands **12** exit0. Minimum exact-archive source gate skutečný child **exit0**, včetně druhých10 distribution, release/checksum/extract/install/help. Stable gate došel final PASS, ale jeho vnější shell capture měl quoting chybu a skončil1; neuvádí captured full stable gate exit0. Kernel commit **2/2** se čtyřmi child0/42 scénáři a negative optional77/required1 gates PASS.
- Jeho kompletní live smoke **neprošel**: první pokus 30s `podman info` timeout, druhý 60s `export-delete-fault` timeout. Následná samostatná actual-OCI EACCES fixture dosáhla správného výsledku za **3,75 / 3,23 s**, bez doložené produktové vady; failed whole-gate pokusy zůstávají failed. Samostatné OCI fixtures doložily UID/GID1000, 0700, nulové capabilities, NoNewPrivs1/Seccomp2, RO root, writable project/private home/state/home sentinel, env/no-login, skutečné default a bounded resource účinky včetně OOM137, native starty, typed-invalid celé store, argv/streams/signály/PTY, syntetický Codex writer/OpenCode account, concurrency/revocation, stopped-only clean a restart race. K4 ponechal1 za neuzavřený vlastní kompletní live gate a chráněnou bezpečnostní validaci. Samostatné PASS nejsou jeho blanket gate PASS. Vlastní procesy i labelované kontejnery:0.
- Web/distribution kritik: původní distribution **10**, web commands **12**, skutečný stable package/literal README/TUTORIAL/checksum/PATH/help a minimum Rust1.85 source prefix-spaces install PASS. Bootstrap INT130 zachoval předchozí instalaci a foreign Cargo; poškozený package checksum správně odmítnut. Pozdní native IO failure naopak FAIL výše. Vlastní pinned image build/checksumy, skutečný embedded recipe přes ac init s neplatným projektovým Dockerfile, osm agent starts a dvě shell disposable/project effect sessions PASS. Stable release SHA-256 **`a84307b5426f5943ccf89426ddc5042982896ae9b4b44874eede109f600c07dd`**, minimum **`3f304d5fc2edb2f6380e8ffdf03fcc0bf0d099a6e551cac75b007405576410d7`**, image **`041b671026c86975483c7d17236c986eb6b0660c637c10c120476631e9dba331`**, recipe **`3b04a60d3524f278464990866d6f59a0fcf39f1b28ed20a1e40b3874723ecd33`**. K4 distribuce snížil také za nedoložené authenticated agent/refresh flow; známka zůstává doslovná.
- Actual Chromium151: **7/7** browser skupin, native grant/denial/unavailable/manual Ctrl+C/recovery/JSoff, mobile320/360/390, actual zoom1/2/4 **1264/632/316** a DPR1/2/4 PASS. Samostatně všechny controls44×44, kontrast nejméně **7,38:1**, skutečné mobilní screenshots čitelné. Dodatečný native-zoom bitmap capture měl headless compositor/clip chybu a nebyl přijat jako vizuální důkaz. Actual Orca heading/method/WhereAmI/command/denial recovery PASS; originální success speech FAIL, kontrolní page PASS výše. Vlastní reader procesy reapnuty.
- Rootovy oddělené minimum/stable exact-archive source gaty **126/126 každý, exit0**, jeho complete actual Podman smoke **17/17, exit0**, včetně nové EACCES a skutečného namespace ENOSPC: tested minimum release SHA-256 **`653eeb2e1cd49923049facf17e1c5e767deef56386ae0db8cfb930395a455409`**. Autorův úspěšný běh nepřepisuje neúspěšné pokusy ani známky nezávislého QA kritika.
- **UNKNOWN:** provider OAuth/login/refresh/authenticated coding a revocation všech čtyř agentů, chráněné bezpečnostní brány, hosted CI/Pages/publishing/current mutable main; jiné runtime/platform/AT kombinace. Žádné syntetické credentials, místní status ani help/version nejsou jejich důkazem. Nová funkční review nebyla retry odmítnutého bezpečnostního workflow.

**Rozhodnutí R5: cíl NEPOTVRZEN.** Jen veřejná dokumentace dosáhla prahu; čtyři nové potvrzené opravy, neuzavřená live brána nezávislého kritika a zásadní provider-auth mezera vyžadují další práci. Původní nároky ani žádná známka nebyly sníženy či upraveny. Další pracovní opravy musí být znovu zmrazeny a nezávisle posouzeny.
