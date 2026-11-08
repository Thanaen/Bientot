# Feature Specification: Plugin runtime (first step)

**Feature Branch**: `003-plugin-runtime`

**Created**: 2026-11-08

**Status**: Draft, ready for planning review

**Input**: The owner asked for a plugin system that offers very wide
customization while Bientôt stays fast. The design was reviewed by five
reviewers (runtime performance, security and privacy, plugin authors,
Microsoft Store, product) over four rounds until they agreed, and ratified as
decision D-024. This specification covers only its first step: prove the
runtime with one read-only flyout card and one palette command, under the
existing size, startup, memory and CPU gates.

## Scope

In scope: the plugin package format and its reader, the trust model, the
WebAssembly host and its budgets, the host interfaces a read-only card needs,
the flyout card slot, plugin commands in the palette, **Settings › Plugins**,
the revocation list, the safe mode, a sample plugin, a minimal SDK, and the CI
gates that keep all of it honest.

Out of scope, each in a later specification: plugin settings, storage,
reminders, timers, no-code automations, themes and tray icon packs, the
forecast note, tray menu entries, plugin pages, the developer mode and its
« Labo » window, network access, secrets, exports, appointment occurrences,
activity sources, reclassification, the public registry and bridges. Network
access, appointment occurrences, activity sources and reclassification each
need their own product decision first (D-024).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Install a plugin and see its card (Priority: P1)

As a user, I install a plugin file I received, see what it will be able to
do, accept, and from then on see its card in the flyout, under the time
already counted. I can turn it off or remove it at any time.

**Why this priority**: it is the smallest journey that exercises the whole
chain: package reader, trust, consent, runtime, host interfaces, card slot and
Settings page. Everything else builds on it.

**Independent Test**: install the sample plugin from **Settings › Plugins ›
Install from file…**, accept, open the flyout, check the card, turn the
plugin off, check that the card is gone, remove the plugin, check that its
folder is gone.

**Acceptance Scenarios**:

1. **Given** no plugin is installed, **When** the user opens **Settings ›
   Plugins**, **Then** the page says no plugin is installed and offers
   **Install from file…**.
2. **Given** a valid signed package, **When** the user picks it, **Then** a
   consent dialog shows the plugin's name, version, author key fingerprint and
   each capability in plain words (« Read your current day », « Show a card
   in the flyout »), with **Install** and **Cancel**.
3. **Given** the user installs it, **When** the next refresh publishes the
   day, **Then** the flyout shows the plugin's card after the counted time,
   never above the departure time, with a header the host draws (plugin
   icon, name, « … » menu).
4. **Given** several plugins have cards, **When** the flyout opens, **Then**
   only one card is visible (the one the user chose last, otherwise the first
   installed) and the others are under **More**.
5. **Given** a card is visible, **When** the user picks **Hide for today** in
   its « … » menu, **Then** the card stays hidden until the next day, like
   the closed callouts.
6. **Given** an installed plugin, **When** the user turns it off, **Then** its
   card and commands disappear within one second and no plugin code runs for
   it until it is turned on again.
7. **Given** an installed plugin, **When** the user removes it, **Then** its
   folder and its state are deleted, and nothing it showed remains.
8. **Given** an unsigned package, **When** the user picks it, **Then** the
   consent dialog warns that the author cannot be verified and that the
   plugin will never update; installing stays possible.
9. **Given** the same plugin id is already installed with another signing
   key, **When** the user picks a package for it, **Then** the install is
   refused with a message that names both fingerprints.
10. **Given** the language is French or English, **When** a card shows,
    **Then** the host's texts follow the app language, and the plugin's texts
    use its locale for that language or its first declared locale.

---

### User Story 2 - Bientôt stays fast and never falls over (Priority: P1)

As a user, I keep the Bientôt I know: same start, same memory, same calm,
whatever a plugin does, and with no plugin it costs nothing.

**Why this priority**: the owner's condition for plugins. A plugin system
that slows the tray app or can crash it is not acceptable.

**Independent Test**: run the Windows performance measurement with no
plugin, with the sample plugin, and with every plugin of the hostile corpus
installed; compare with the release baseline.

**Acceptance Scenarios**:

1. **Given** no plugin is installed, **When** Bientôt starts, **Then** no
   plugin thread starts, no WebAssembly engine is created, and the only extra
   work is reading an empty folder.
2. **Given** plugins are installed, **When** Bientôt starts, **Then** the
   tray is ready before any plugin manifest is read, and no plugin module is
   compiled before its first event.
3. **Given** a plugin loops forever, recurses deeply, grows its memory
   without end or answers with a huge view tree, **When** it is called,
   **Then** the call stops with a trap or a refusal, the process keeps
   running, and the flyout keeps answering.
4. **Given** a plugin traps three times within 10 minutes, **When** the third
   trap happens, **Then** the plugin is turned off until the next start, and
   **Settings › Plugins** says why.
5. **Given** Bientôt crashed twice in a row during startup, **When** it
   starts the third time, **Then** it starts without any plugin and says so
   in the flyout, with a way to turn plugins back on.
6. **Given** the user starts Bientôt with `--no-plugins`, **When** it runs,
   **Then** no plugin is loaded.
7. **Given** a plugin is slow, **When** the flyout or the week renders,
   **Then** they draw the last card the plugin produced; the render never
   waits for plugin code.

---

### User Story 3 - Run a plugin command from the palette (Priority: P2)

As a user, I find a plugin's commands in the command palette, with the
plugin's name in front, and run them.

**Why this priority**: it proves the second kind of contribution (an action
the user starts) and the priority of user actions over background work.

**Independent Test**: install the sample plugin, open the palette with
Ctrl+Maj+P, type part of its command, run it, check that the card changes.

**Acceptance Scenarios**:

1. **Given** a plugin declares a command, **When** the palette opens,
   **Then** the command is listed as « <plugin>: <title> » without the
   plugin being instantiated.
2. **Given** the user runs it, **When** the plugin is busy with a background
   event, **Then** the command runs before the next background slice.
3. **Given** a plugin declares a command with the title of a built-in
   command, or declares a keyboard shortcut, **When** it is installed,
   **Then** the install is refused with a message that says why.
4. **Given** a card button names a command, **When** the user activates it,
   **Then** the same command runs, with the button's argument.

---

### User Story 4 - A bad plugin is stopped (Priority: P2)

As a user, I am protected from a plugin that was found dangerous after I
installed it, or whose files changed on disk.

**Why this priority**: the trust model must exist before the first plugin is
shared, since a sandbox flaw or a malicious plugin cannot wait for an app
release.

**Independent Test**: serve a revocation list that names the sample plugin;
check that it turns off with an explanation. Edit `plugin.toml` of an
installed plugin; check that it no longer loads until it is installed again.

**Acceptance Scenarios**:

1. **Given** « Check for updates » is on, **When** the daily check runs,
   **Then** Bientôt also reads the signed revocation list, sending no
   identifier, and turns off every installed plugin the list names, with the
   reason it gives.
2. **Given** the list sets `level2_disabled`, **When** it is read, **Then**
   every WebAssembly plugin turns off until a later list clears the flag.
3. **Given** the user installs a plugin or turns one on, **When** they do,
   **Then** Bientôt reads the list first, on every channel, the portable ZIP
   included; if it cannot be reached, it uses the last list it saw and says
   so.
4. **Given** a list older than the last one seen, or past its expiry,
   **When** it is read, **Then** it is ignored.
5. **Given** any file of an installed plugin changed, **When** the plugin
   loads, **Then** it is refused, and **Settings › Plugins** offers to
   remove it.

---

### User Story 5 - Build the sample plugin (Priority: P3)

As a plugin author, I build, test, pack and sign the sample plugin from the
repository, with documented commands.

**Why this priority**: the runtime must be exercised by a real plugin built
the supported way; the developer mode and its tools come in a later step.

**Independent Test**: from a clean clone, follow `quickstart.md`, produce a
signed `.bientot-plugin`, install it in a `--demo` run.

**Acceptance Scenarios**:

1. **Given** the SDK crate, **When** the author builds the sample plugin for
   `wasm32-unknown-unknown`, **Then** it links with a 64 KiB WebAssembly
   stack and imports only the interfaces of its declared capabilities.
2. **Given** the pack tool, **When** a module imports an interface that no
   declared capability allows, **Then** packing fails and names the import
   and the capability to add.
3. **Given** the SDK's native test host, **When** the author runs
   `cargo test` on the plugin, **Then** the plugin's handlers run against
   the demo scenarios' day snapshots with an injected clock.

### Edge Cases

- A package with two entries of the same name, entries differing only by
  case, `..`, an absolute path, a drive letter, a UNC path, a `:`, a reserved
  Windows name, a trailing dot or a symbolic link: refused.
- A package bigger than 8 MiB, with more than 256 entries, or with a
  compression ratio above 100: refused.
- A manifest with an unknown key, a bad id, an `api` this host does not
  support, or an import outside the allowed list: refused, with a message
  that names the problem.
- A view tree deeper than 6, with more than 64 nodes, a text longer than 200
  characters, or a primitive the declared `api` does not have: rejected and
  counted as a failure.
- A card longer than 4 lines: cut to 4 lines by the host.
- The plugin folder is copied by hand into `%LOCALAPPDATA%\Bientot\plugins`
  on the Store build: ignored, since the Store build reads its redirected
  folder and the plugin is not in its state file.
- Bientôt runs on a slow PC: a call that hits the 500 ms wall-clock safety
  net stops but does not count as a failure.
- The memory pool is full: idle instances are evicted first; a growth that
  still fails is a trap that does not count as the plugin's failure.
- The PC sleeps while a plugin call runs: the call is abandoned; the plugin
  gets the next structural change after resume.
- The user changes the app language: plugins get `language.changed` and the
  card is redrawn in the new language.
- A transfer file is exported: it carries the list of installed plugins and
  never their files, state or granted capabilities.

## Requirements *(mandatory)*

### Functional Requirements

**Packages and trust**

- **FR-001**: Bientôt MUST install plugins only from a `.bientot-plugin`
  file the user picks or drops on **Settings › Plugins**.
- **FR-002**: The package reader MUST NOT use generic extraction. It MUST
  read the entries into memory, accept only the names in the package
  contract, apply every refusal of the Edge Cases, and write the accepted
  files to `<data folder>\plugins\<id>\`.
- **FR-003**: A signature MUST cover a canonical list of (path, SHA-256)
  pairs, not the zip bytes. On first install the plugin id MUST be bound to
  the signing key; later packages for that id MUST be signed by the same key
  or by a key the previous key announced.
- **FR-004**: An unsigned plugin id MUST be bound to its package hash; any
  replacement MUST count as a new install.
- **FR-005**: Bientôt MUST check every file's hash against the signed list
  each time a plugin loads, not only at install.
- **FR-006**: Granted capabilities MUST be bound to the manifest hash; a
  changed manifest MUST ask for consent again.
- **FR-007**: The consent dialog MUST name each capability in plain words,
  the author key fingerprint (or that the package is unsigned), and the
  memory the plugin declares when above the default.

**Runtime**

- **FR-010**: Bientôt MUST run plugins with `wasmi` pinned to an exact
  version, with `default-features = false` and the features `std`,
  `validate` and `portable-dispatch`.
- **FR-011**: Plugin code MUST run only on a dedicated `plugins` thread,
  created when the first plugin is turned on, never on the GPUI thread and
  never inside the accounting refresh.
- **FR-012**: The queue to that thread MUST be bounded, coalesce day
  snapshots (the latest wins) and serve user actions first.
- **FR-013**: Each call MUST run in slices of 2 M fuel with resumable calls,
  with a fuel allowance of 20 M plus 250 per byte of input, capped at 100 M
  for background events and 400 M for user actions, and a wall-clock safety
  net of 500 ms (2 s for user actions) that does not count as a failure.
- **FR-014**: Each plugin MUST have a token bucket of 20 ms per minute with a
  500 ms reserve, and all plugins together one of 60 ms per minute with a 1 s
  reserve; host work done for a plugin counts. An empty bucket delays the
  plugin's background events.
- **FR-015**: Memory MUST be limited to 8 MiB per instance by default, 32 MiB
  at most on declaration, and 32 MiB for all plugins. Only the declared
  minimum (1 MiB by default) is reserved at activation. A failed growth
  evicts idle instances and retries once.
- **FR-016**: After a trap the instance MUST be discarded. Three traps within
  10 minutes MUST turn the plugin off until the next start.
- **FR-017**: An instance with no event for 5 minutes MUST be evicted; its
  compiled module MAY stay. An instance whose memory exceeds max(2 × its size
  after `activate`, 1 MiB) MUST be re-created between events.
- **FR-018**: With no plugin turned on, Bientôt MUST NOT start the plugin
  thread or create a WebAssembly engine.
- **FR-019**: The host MUST read plugin manifests only after the tray is
  ready, and compile a module only before its first event.
- **FR-020**: The module's imports MUST be checked against the granted
  capabilities at install and at every instantiation; an unknown import MUST
  refuse the module.

**Host interfaces of this step**

- **FR-030**: Bientôt MUST offer the interfaces `bt:log@1`, `bt:time@1`
  (`now` only), `bt:day@1` and `bt:ui@1` (flyout card slot only), and no
  other import.
- **FR-031**: The day snapshot MUST be a hand-written, versioned type, built
  once per refresh and only when a plugin listens, carrying the target, the
  counted time, the departure at the target and at the minimum, the tracking
  state, the settled segments and the open tail with public kinds, the week
  credit and the app language. It MUST NOT carry domain types serialized as
  they are.
- **FR-032**: Bientôt MUST send `day.refreshed` only on a structural change:
  a segment settled, the tracking state, the target, or a departure that
  moved by one minute or more; at most once per 30 s.
- **FR-033**: Bientôt MUST send `installed` and `upgraded` once, `activate`
  at every instantiation with the language, `command.invoked` for commands
  and buttons, and `language.changed`.
- **FR-034**: Plugin log lines MUST go to a per-plugin file under
  `logs\plugins\`, at most 1 MiB and 14 days, rate-limited, with the text in
  a field that the redaction keeps, and MUST NOT enter the diagnostic export.

**Interface surfaces**

- **FR-040**: The flyout MUST show at most one plugin card, after the
  counted time, of at most 4 lines plus one row of at most 2 buttons, with a
  header the host draws; the other cards go under **More**.
- **FR-041**: Cards MUST be built from the view-tree primitives of the
  contract and drawn with the host's components and tokens; a `Duration` or
  `Countdown` MUST advance without waking the plugin.
- **FR-042**: A card update MUST NOT be announced to the screen reader;
  focus MUST return to a sensible place when a card disappears.
- **FR-043**: Plugin commands MUST appear in the palette as « <plugin>:
  <title> », without instantiating the plugin; built-in titles and keyboard
  shortcuts MUST be refused.
- **FR-044**: **Settings › Plugins** MUST list installed plugins with their
  version, author fingerprint, capabilities, state (on, off, turned off after
  failures, revoked, files changed) and actions (turn on or off, remove,
  open the plugins folder at its real path).

**Revocation and safe mode**

- **FR-050**: Bientôt MUST read a signed `revocations.json` from
  `bientot.dev` with the daily update check, and when the user installs or
  turns on a plugin, sending no identifier.
- **FR-051**: The list MUST carry a version that only increases and an
  expiry date; an older or expired list MUST be ignored.
- **FR-052**: `--no-plugins` MUST start Bientôt without any plugin; two
  crashes in a row during startup MUST start the next run without plugins.

**Data and privacy**

- **FR-060**: Installed plugins and their state MUST live in
  `<data folder>\plugins\`; removing a plugin MUST delete its folder.
- **FR-061**: The transfer file MUST carry only the ids and versions of the
  installed plugins.
- **FR-062**: `docs/privacy.md` MUST list the revocation list request and the
  new files and folders.

**Build and CI**

- **FR-070**: Everything in this step MUST be behind a `plugins` Cargo
  feature of `bientot-app`, on in `release` and `direct`.
- **FR-071**: CI MUST measure the executable size of both `release` and
  `direct` builds against the 20 MB gate.
- **FR-072**: CI MUST run a corpus of hostile plugins against the Windows
  release build and fail on any process abort.
- **FR-073**: CI MUST fuzz the manifest parser, the package reader, the host
  call decoding and the view-tree validation.

### Key Entities

- **Plugin package**: the `.bientot-plugin` file: manifest, module, locales,
  assets, signature.
- **Manifest**: id, name, version, `api`, declared memory, locales,
  capabilities, events, commands, slots.
- **Installed plugin**: id, version, trust binding (key fingerprint or
  package hash), file hashes, granted capabilities and the manifest hash they
  were granted for, on or off, failure record.
- **Revocation list**: version, expiry, revoked ids or keys with reasons,
  `level2_disabled`, signature.
- **Day snapshot**: the published day as plugins see it.
- **View tree**: a card as the plugin describes it; **prepared card**: the
  same after validation, ready to draw.
- **Contribution set**: every card and command currently published, replaced
  as a whole.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The `release` and `direct` executables each grow by at most
  1.0 MB and stay under 20 MB.
- **SC-002**: With no plugin, startup to `startup.tray_ready`, idle working
  set and idle CPU stay within the measurement noise of the previous release
  (0.30 s, 66.3 MiB, 0.013 %).
- **SC-003**: With the sample plugin on, the idle working set grows by at
  most 2 MiB and idle CPU stays under the 0.5 % budget.
- **SC-004**: Across the whole hostile corpus on the Windows release build,
  the process never aborts, and its working set returns within 5 MiB of its
  value before the corpus once the instances are dropped.
- **SC-005**: A card reflects a structural change of the day within 30 s
  plus one call.
- **SC-006**: The fuzz targets run for 60 s on each pull request that
  touches them, and 10 minutes every week, with no panic.
- **SC-007**: Narrator reads the card header, its lines and its buttons, in
  French and in English.
- **SC-008**: A user installs the sample plugin, sees its card and runs its
  command in under one minute, without reading documentation.

## Assumptions

- The sample plugin shows a « Focus » card: the longest continuous active
  period today, the active time since the last break, and the departure
  time, with a button that switches between today and the week.
- Plugins of this step need no network, settings, storage or timers; they
  react to `day.refreshed` and commands.
- The revocation list starts empty, signed with an offline key held by the
  maintainer; `services/bientot` serves it as a static file.
- The Microsoft Store listing gains its plugin line and certification notes
  when the first build with plugins is submitted.
- Design system and mockup artifacts are updated with the card, the consent
  dialog and **Settings › Plugins** when the UI is built, per the project's
  design rules.
