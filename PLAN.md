# mattertui — plan

A read-only terminal dashboard for the Matter devices on my network: smart plugs,
thermostats and battery temperature sensors. Some are on Wi-Fi and some are on Thread
behind a HomePod mini border router. Devices are grouped into rooms and laid out
automatically, with live graphs and lots of colour and movement.

**One binary, no daemon.** Running `mattertui` shows the dashboard. A few
subcommands handle one-off jobs (§9). macOS and Linux (Raspberry Pi in tmux)
behave the same way, with no platform-specific code paths. There's no
database: pairings are saved (§2), and graph history is in memory only.

**Core crates:** `ratatui`, `matter-controller` 0.18.3, `tokio`, `anyhow`.

---

## 1. Pairing (a setup code is the only input)

The devices stay in Apple Home. mattertui joins each one as a **second controller**
using Matter's Multi-Admin feature.

1. In the Home app, open the device and go to Settings → **Turn On Pairing Mode**.
   Copy the 11-digit code.
2. In mattertui, select the device's tile (shown as *unpaired*), press `p` and
   enter the code. Both forms are accepted:
   - the manual code, e.g. `3497-011-2332`, with any dashes or spaces
   - a QR payload, `MT:…`

   The check digit is validated as you type.
3. The app runs `controller.commission(code, Some(device_id))` over IP; no BLE is
   needed. Thread devices are reached through the HomePod. `commission()` is a
   single call with no progress callbacks, so the modal shows a spinner. The
   library's own `tracing` output streams into the log panel as it goes.
4. It then sets the fabric label to `"mattertui"`, so it's easy to spot in Home →
   Connected Services.

The commissioning `label` is the config's **device id**. That's the only link
between config and controller state, so the config never contains node IDs.

**Pairing happens once.** The result is saved (§2). On the next launch the app
reconnects to every paired device without asking for a code. Pairing from the
CLI works the same way: `mattertui pair <device-id> <code>`.

**Unpair:** select the tile and press `U`, or run `mattertui unpair <device-id>`.
This removes our fabric from the device, then calls `forget_node`. If the device
can't be reached, the app only forgets it locally.

**Read-only:** device workers get a `ReadOnlyNode` wrapper that only exposes
`read` and `subscribe`. Only `pairing.rs` can commission, set the fabric label or
remove fabrics.

**Attestation roots:** retail devices need the production PAA and CD certificates
from connectedhomeip. `mattertui fetch-roots` downloads them into the data
directory. It uses the GitHub contents API to list
`credentials/production/{paa-root-certs,cd-certs}`, then fetches each `.der`
file. Re-running it picks up new vendors. If the roots are missing, the pairing
modal tells you to run it.

---

## 2. Configuration

### Where things live (the same on macOS and Linux)

| What | Path | Who writes it |
|---|---|---|
| Config | `$XDG_CONFIG_HOME/mattertui/config.toml` (default `~/.config/…`); `--config` overrides | you |
| Pairing state | `$XDG_DATA_HOME/mattertui/controller.bin` (default `~/.local/share/…`) | matter-controller's `FileStore` |
| Attestation roots | `$XDG_DATA_HOME/mattertui/roots/{paa,cd}/` | `mattertui fetch-roots` |

XDG paths are used on macOS too, not `~/Library/Application Support`, so both
platforms behave the same. This is about 10 lines of our own code, so it needs no
paths crate.

### What `controller.bin` holds

It's the library's own snapshot format, verified in the crate source:

- the fabric's root certificate and key
- our controller's operational identity (NOC and key)
- the IPK
- for each paired device:
  - node id
  - public key
  - **label** (= config device id)
  - vendor/product id
  - last known address
  - CASE resumption record, for fast reconnects

Saves are atomic (temp file then rename) and the file mode is `0600`. **It holds
private keys in the clear.** Anyone who copies it can act as this controller on
your devices. The data dir is created as `0700`. Back the file up if you don't
want to re-pair after a disk loss. Copying it from the Mac to the Pi moves every
pairing across, as long as only one machine runs at a time.

### Config

The config holds **no coordinates**. Rooms and devices appear in the order they're
declared, and the layout engine does the rest.

```toml
[ui]
temperature_unit = "celsius"   # celsius | fahrenheit
motion = true                  # false = no ambient animation
history_hours = 24             # in-memory graph window

[log]
panel = "hidden"               # hidden | open — state at startup; toggle with `
height = 8                     # lines when open
filter = "info,mdns_sd=warn"   # tracing EnvFilter syntax; RUST_LOG overrides

[[room]]
id = "lounge"
name = "Lounge"
icon = "sofa"
accent = "amber"               # optional; assigned from the palette otherwise

[[room]]
id = "bedroom"
name = "Bedroom"
icon = "bed"

[[device]]
id = "lounge-lamp"             # stable key = commissioning label
name = "Floor lamp"
room = "lounge"
kind = "plug"                  # plug | thermostat | temperature_sensor
icon = "lamp"

[[device]]
id = "hall-thermostat"
name = "Heating"
room = "lounge"
kind = "thermostat"
featured = true                # large tile with big readout and chart

[[device]]
id = "bedroom-temp"
name = "Bedside"
room = "bedroom"
kind = "temperature_sensor"
```

**Validation on startup** (also available as `mattertui check`): problems are
printed and the app exits before the TUI starts. Errors include line and column numbers. These are checked:
- duplicate ids, or ids with characters outside `[a-z0-9-]`
- a `room`, `kind` or `icon` that doesn't exist

Paired nodes with no matching config entry show in an "Unassigned" panel, with
their node id and vendor/product, so you can add them to the config.

---

## 3. What each kind reads

There is **one subscription per device**. On every startup the app reads
Descriptor (0x001D) to find the right endpoint. That's a single read, so
nothing is cached.

| Kind | Device type | Metrics |
|---|---|---|
| plug | 0x010A (or 0x0100) | `on` from OnOff 0x0006/0x0000 |
| thermostat | 0x0301 | from Thermostat 0x0201:<br>• `temperature` LocalTemperature 0x0000<br>• `heat_setpoint` 0x0012<br>• `cool_setpoint` 0x0011<br>• `mode` SystemMode 0x001C<br>• `running` ThermostatRunningState 0x0029 |
| temperature_sensor | 0x0302 | • `temperature` from 0x0402/0x0000<br>• `humidity` from 0x0405/0x0000, if present |
| any | — | • `battery` from Power Source 0x002F/0x000C<br>• Wi-Fi/Thread badge from Network Commissioning 0x0031 FeatureMap<br>• vendor, product and firmware from Basic Information 0x0028 |

Temperatures are int16 values in units of 0.01 °C and may be null. Decoding is
unit-tested against captured values.

---

## 4. In-memory history

Each graphed metric (temperature, humidity, setpoint, `on`, `running`) gets a
fixed-size ring buffer of **1-minute buckets**, each holding min, max and last
value. With `history_hours = 24` that's 1,440 buckets per metric, which is a few
KB per device. The buffers start empty at launch and are lost when the app quits.

Charts offer `15m · 1h · 6h · 24h`, capped at how long the app has been running.

---

## 5. Battery sensors (ICDs)

Battery Thread sensors are *Intermittently Connected Devices*: they sleep most of
the time. Most temperature sensors are the **short idle time** (SIT) kind, where
a normal subscription works; the device just negotiates a long `max_interval`.

- **Staleness:** a tile only goes stale after twice the device's negotiated
  `max_interval` plus 30 s, never a fixed timeout.
- **No check-in support (decided 2026-11-15).** The battery sensor (IKEA
  TIMMERFLOTTE, Thread) is SIT with no check-in support, idling up to 300 s,
  and plain subscriptions work. So the app neither registers as a check-in
  client nor runs a listener.
- **If a long idle time (LIT) device turns up:** `mattertui nodes --probe`
  prints each device's ICD type. LIT devices may drop subscriptions while
  asleep. Supporting one would mean registering at pairing
  (`Node::register_icd_client`) and a background
  `listen_for_checkin_once` → `resubscribe_now` loop.

---

## 6. Auto-layout

Layout is a pure function:
`layout(rooms, devices, area) -> Vec<(Id, Rect, Variant)>`.
It's snapshot-tested at several terminal sizes.

1. **Tile size from kind:**

   | Kind | Compact (1 line) | Normal | Featured |
   |---|---|---|---|
   | plug | yes | 3 lines with on-timeline | 3 lines with on-timeline |
   | sensor | yes | 4 lines with sparkline | big readout and chart |
   | thermostat | yes | 5 lines | big readout, setpoint gauge, chart with heating bands |

2. **Room panel:** a header with name, icon and summary (average temperature,
   "2/3 lights on"), with tiles flowing in 1–2 columns depending on width.
3. **Masonry packing:** column count is `width / min_room_width`. Each room goes,
   in declaration order, into the shortest column.
4. **Fitting:** if it doesn't fit, featured tiles become normal, then normal tiles
   become compact, then the view scrolls.

On resize the layout is recomputed and tiles animate to their new positions.

---

## 7. Look and movement

**Colour**
- One dark truecolor palette, with a 16-colour fallback.
- Each room has an accent colour for its border, title and chart lines.
- Temperatures use a 5-stop gradient (blue → teal → green → amber → red). It
  applies to numbers, sparklines and chart lines. This is a small hand-written
  interpolation, not a crate.

**Movement** (hand-rolled in `ui/motion.rs`; `tachyonfx` turned out not to be needed)
- **Startup:** rooms sweep in left to right, one after another, behind a scan
  line in the room's accent colour.
- **Updates:** values tween to the new number (700 ms, ease-out) and the tile
  border flashes in the room accent.
- **Plugs:** "ON", the plug's icon colour and the room's lights count breathe
  gently in warm amber.
- **Heating:** the "heating" label flickers between flame colours.
- **Connecting:** a braille spinner in the tile badge.
- **Ticker:** a bottom line of recent changes (on/off, heating, moves of ≥0.5°,
  offline / back online, paired). It scrolls as a marquee when it overflows.
- **`m` / `ui.motion = false`** turns all of it off.

**Chrome**
- **Top bar:** title, devices live/total, lights on, clock.
- **Bottom ticker:** recent events, e.g. "Lounge lamp on · 2m" or
  "Bedroom 18.4° ↓".

**Log panel** (toggle with `` ` ``; startup state comes from `[log] panel`)
- **Placement:** docked at the bottom, `[log] height` lines tall. When it's open,
  the layout engine gets less height and reflows the rooms. The panel slides
  in and out.
- **Content:** a live stream of `tracing` events from the app and from
  `matter-controller`: commissioning steps, subscription status,
  resubscribes and errors.
- **Line format:** `HH:MM:SS LEVEL target message`.
  - Levels are coloured: ERROR red, WARN amber, INFO cyan, DEBUG/TRACE dim.
  - When a line carries a `device` field, the device name is shown in its
    room's accent colour.
  - New lines briefly highlight.
- **Scrolling:** it follows the newest line by default. `Tab` focuses the panel.
  Then `↑↓` / `PgUp` / `PgDn` scroll and pause following, and `End` resumes it.
- **Alerts while hidden:** if a WARN or ERROR arrives while the panel is hidden,
  a badge appears in the top bar (e.g. `⚠ 2`) so problems don't go unnoticed.
  Opening the panel clears it.

**Detail view (`Enter`):** a large braille chart with a range switcher, plus
min/avg/max, vendor, product, firmware, transport, node id and subscription
health.

**Performance:** 30 fps while something is actively animating, 10 fps for
ambient motion (breathing lights, flicker, a scrolling ticker), otherwise one
redraw per second for the clock and "ago" labels. On the Mac a release build
used about 2.6% of one core with a light on.

**Fallback:** the same code runs on both platforms.
- Icons are Unicode symbols (`◉ ☼ ♨ ❄ ⌂`), not nerdfont or emoji. They have
  consistent widths in tmux and need no special font.
- If `TERM=linux` (the bare console), the app uses ASCII icons and block charts.
- If `COLORTERM` isn't `truecolor`/`24bit`, it uses the 16-colour palette.

**Keys**

| Key | Action |
|---|---|
| arrows / `hjkl` | move between tiles |
| `Enter` | device detail |
| `[` `]` | change chart range |
| `p` | pair |
| `U` | unpair |
| `m` | toggle motion |
| `` ` `` | toggle log panel |
| `Tab` | focus the log panel (when open) |
| `?` | help |
| `q` | quit |

---

## 8. Architecture

```
main ─► load+validate config ─► build MatterController ─► spawn tasks ─► run TUI

tasks ── DeviceWorker × N : subscribe → decode → AppEvent::Update ──┐
      ── IcdListener      : check-in → resubscribe_now               │ mpsc
      ── Pairing (on demand): commission → label → ICD reg ──────────┤
                                                                      ▼
App loop: select!{ terminal events, AppEvent, frame tick (only while animating) }
          state: devices + history rings  →  layout()  →  widgets + effects  →  draw
```

The TUI owns all state. Workers only send `AppEvent`s, and there is no protocol
layer.

### Logging (no file)

Logs go to stderr or to a panel, never to a file.

- **Dashboard:** a small custom `tracing_subscriber::Layer` (`logbuf.rs`, about
  60 lines) formats each event into a styled line.
  - Lines go into a bounded ring buffer (1,000 lines), shared as
    `Arc<Mutex<VecDeque<LogLine>>>`.
  - Each new line also sends `AppEvent::Log`, so the panel redraws only when
    something arrives.
  - It's hand-rolled rather than `tui-logger`, so the lines get the same
    styling and colours as the rest of the dashboard.
- **Subcommands** (`pair`, `check`, …): a plain `fmt` layer writes to stderr.
- **On exit or panic:** a panic hook restores the terminal first. If the app
  exits with an error or panics, it then prints the last 50 buffered lines to
  stderr, so the context isn't lost when the TUI closes.
- **Filter:** `[log] filter` uses `EnvFilter` syntax, and `RUST_LOG` overrides
  it.

**DeviceWorker** maps subscription events like this:

| Event | Action |
|---|---|
| `Report` | update the value and its history |
| `Established` | mark live; store `max_interval` |
| `Resubscribing` | mark reconnecting |
| `Lagged` | re-read everything |

If the first subscribe fails, it retries with backoff from 1 s up to 5 min.

**Single instance:** the library allows only one process per controller identity.
A second `mattertui` would cancel the first one's subscriptions. So every
command that opens the controller (the dashboard, `pair`, `unpair`) takes a
`std::fs::File::try_lock` on `controller.lock`. That API is stable since Rust 1.89,
which matter-controller already requires. If the lock is held, the command exits
with an "already running" message; for `pair` it adds "press `p` in the dashboard instead".

### Subcommands (`clap`)

| Command | Does |
|---|---|
| `mattertui` | dashboard |
| `mattertui pair <device-id> <code>` | commission one device from the CLI (same code path as `p`) |
| `mattertui unpair <device-id>` | remove our fabric from the device and forget it |
| `mattertui nodes` | list paired nodes: label, node id, vendor/product, and whether a config entry matches |
| `mattertui check` | validate the config and exit |
| `mattertui fetch-roots` | download the PAA/CD attestation roots |

Every command accepts `--config <path>`.

```
src/
  main.rs         clap subcommands, startup, terminal setup/restore
  paths.rs        XDG dirs, 0700 data dir, instance lock
  config.rs       types, validation
  roots.rs        fetch-roots (GitHub contents API → .der files)
  matter/
    mod.rs        controller bootstrap, clock check
    readonly.rs   ReadOnlyNode
    probe.rs      endpoint detection
    worker.rs     subscription loop
    decode.rs     Value → metrics
    icd.rs        check-in listener
    pairing.rs    commission, label, ICD registration, unpair
  state.rs        DeviceState, History ring
  ui/
    app.rs        event loop, keys
    layout.rs     auto-layout
    theme.rs      palette, gradient, capability detection
    motion.rs     easing, tweens, colour mixing, ticker
    logbuf.rs     tracing Layer → ring buffer + AppEvent::Log
    widgets/      room, tile_plug, tile_sensor, tile_thermostat, chart, ticker, top_bar, log_panel, pair_modal, detail
```

**Crates**

| Crate | Why |
|---|---|
| `ratatui`, `crossterm` (`event-stream`) | TUI |
| `tokio`, `futures` | runtime and `EventStream` |
| `anyhow` | errors |
| `matter-controller` | Matter |
| `serde`, `toml` | config |
| `clap` (derive) | subcommands |
| `ureq` (rustls), `serde_json` | `fetch-roots` only; blocking, so no async HTTP stack |
| `tui-big-text` | big readouts |
| `tracing`, `tracing-subscriber` (`env-filter`) | in-app log panel; stderr for subcommands |
| `insta` (dev) | layout and widget snapshot tests |

---

## 9. Network prerequisites

The app code is identical on both platforms. Only OS-level network setup differs.

- **Thread routing:** the host needs the IPv6 route the HomePod advertises.
  - macOS accepts it automatically.
  - Linux needs `net.ipv6.conf.wlan0.accept_ra_rt_info_max_plen=64` (persist it
    in `/etc/sysctl.d/`). Check with `ip -6 route | grep fd`.
- **Clock:** the Pi has no RTC. If the clock isn't set yet, the app shows
  "waiting for clock sync" instead of failing commissioning.
- **mDNS:** the library's pure-Rust responder works alongside both Bonjour and
  avahi.
- **Matter uses IPv6.** Use `ping -6` against the addresses from
  `avahi-browse -r`.

---

## 10. Milestones

1. **Spike** (done, on the Mac):
   - Build the real `paths`, `fetch-roots`, `pair` and `nodes` subcommands.
   - Pair one Wi-Fi plug and one battery Thread sensor, then **restart and
     confirm both reconnect with no code**.
   - Read values and check the sensor's ICD type.
2. **Engine without UI** (done): config and `check`, workers, decoding,
   history, `unpair`, single-instance lock.
3. **Static dashboard** (done): log panel, layout and snapshot tests, rooms,
   tiles, detail chart, theme and fallback.
4. **Pairing and unpairing in the TUI** (done). The ICD listener was not
   needed (see §5).
5. **Motion** (done): frame clock, tweens, flashes, sweep, breathing, ticker.
6. **Thermostats** (done, §11): validated against a tado° X; heating is
   estimated because the device doesn't report it.
7. **Visual refresh** (done, §12): btop-inspired graphs, panels that fill
   the screen, one orange + teal palette, tighter alignment.
8. **btop fidelity** (done, §13): look like btop's default theme, following
   the reference screenshot.
9. **Polish and persistence** (§14): never-emoji icons, saved history, a
   white/teal/orange scheme, floating boxes in featured rooms, subtle
   gradients, and a setpoint line.

---

## 11. Milestone 6: Thermostats

**Result (2026-11-16).** The tado° (it identifies as "Wireless Temperature
Sensor X", fw 1.4.289) exposes only these thermostat attributes:
LocalTemperature, Abs Min/Max HeatSetpointLimit (5–30 °C),
OccupiedHeatingSetpoint, ControlSequenceOfOperation (heating only) and
SystemMode. It's SIT (idle 120 s), with no check-in support, humidity on
endpoint 2, and battery as BatChargeLevel only.

What was built:
- Humidity for thermostats.
- Battery charge level as a fallback when there's no percentage
  (ok / low / replace).
- PIHeatingDemand, when present.
- Capabilities and limits read once at discovery.
- Heating **estimated** from temperature vs setpoint (on 0.2° below,
  off 0.1° above) when neither running state nor demand is reported. The
  detail view labels it as an estimate.
- Heating-only devices hide cooling and the redundant "heat" mode.
- A clear "off" state.
- A setpoint gauge on featured tiles.
- 🔥 in the room summary while heating.
- Ticker entries for setpoint and mode changes.
- `nodes --probe` dumps every Thermostat and PowerSource attribute.

Not built, because the tado doesn't expose them: occupancy and unoccupied
setpoints, outdoor temperature, running mode, setpoint change source, and
presets. The probe will show them if a future device has them. Bridged
thermostats are covered by the existing `endpoint` override.

The original plan follows.

**Already in place (untested on hardware):**
- `kind = "thermostat"`.
- Endpoint detection by device type 0x0301.
- Subscriptions to LocalTemperature, heat/cool setpoints, SystemMode and
  ThermostatRunningState, filtered by AttributeList.
- Normal and featured tiles, a setpoint line on the chart, and a "Running"
  timeline in the detail view.

**Target device: tado° Wireless Smart Thermostat X.** A battery-powered
wall thermostat on Thread, so it's reached through the HomePod like the
TIMMERFLOTTE. Expect:
- **It's probably an ICD.** Check the probe's `icd:` line before anything
  else.
- **It may also report humidity.** If the node has a Relative Humidity
  cluster (0x0405), the thermostat tile shows it, the same as sensors do
  today. The current discovery only adds humidity for `temperature_sensor`.
- **Heating demand matters more than running state.** It controls the
  boiler through tado's own receiver, so PIHeatingDemand and running state
  may be what reflects "heating".

**Plan:**

1. **Pair and probe a real thermostat first.** Run
   `mattertui pair <id> <code>`, then `nodes --probe`. Extend `--probe` to list
   every Thermostat (0x0201) attribute the device exposes, with its value.
   The rest of this milestone is driven by what the device actually has.
   Capture the probe output as a test fixture.
2. **Read the attributes real devices commonly expose:**
   - **ControlSequenceOfOperation** (0x001B): heat-only, cool-only or both.
     Decides which setpoints and modes to show, so a heat-only radiator valve
     never shows a cooling setpoint.
   - **Heat setpoint limits** (0x0015 and 0x0016): the scale for a setpoint
     gauge.
   - **PIHeatingDemand** (0x0008, percent): valve or boiler demand. Use it to
     infer "heating" when ThermostatRunningState (0x0029) is missing, which
     it often is on radiator valves. Graph it in the detail view.
   - **OutdoorTemperature** (0x0001), if present.
   - **Occupancy** (0x0002), and the unoccupied setpoints
     (0x0013 and 0x0014).
   - **ThermostatRunningMode** (0x001E) and **SetpointChangeSource**
     (0x0030), if present.
   - **Matter 1.4 presets:** the active preset name (e.g. "Away" or "Eco"),
     if the device has them.
3. **Display:**
   - Featured tile: a setpoint gauge showing the min–max limits, the current
     temperature marker and the setpoint marker. This was planned in §6 but
     never built.
   - Heating demand as a small meter.
   - A clear "off" state when SystemMode is off: dimmed, with no setpoint.
   - The room summary shows 🔥 while any thermostat in the room is heating.
4. **Ticker:** "Heating set to 21.0°" when the setpoint changes, plus mode
   changes ("Heating off", "Heating → auto").
5. **Battery thermostats:** radiator valves are often Thread ICDs. Run the
   probe's `icd:` check. If one is a long idle time (LIT) device, revisit
   the check-in listener (§5).
6. **Bridged thermostats** (e.g. via a vendor's Matter bridge): make sure
   endpoint selection copes with several thermostat endpoints behind one
   node, using the config's `endpoint` override.
7. **Tests:** decoding of every new attribute, the inferred running state,
   heat-only versus heat+cool display, and snapshots of each thermostat
   tile size using the captured fixture.

Still read-only: no setpoint or mode changes.

---

## 12. Milestone 7: Visual refresh (btop-inspired)

**Result (2026-11-16):**
- **Palette:** teal and blue for structure, orange for emphasis, a
  temperature gradient running blue → teal → amber → red, and a teal tint
  per room. Tuned for a black background; the app paints none.
- **Graphs:** braille area graphs (`ui/graph.rs`) coloured by height, with
  a setpoint overlay line, and `■` meters.
- **Tiles:** borderless sections inside each room box, with a header rule
  that flashes on updates, readouts on a fixed grid, and graphs that absorb
  spare rows.
- **Plugs:** they grow too; their spare rows show a square-wave on/off
  graph, plus "on for 25m".
- **Layout v2:** columns fill the full height, with spare rows shared by
  graph weight.
- **Chrome:**
  - room titles `┤¹ 🏠 Living room├` with `1`–`9` to jump;
  - a summary that drops items so the room name is never cut;
  - a top rule with the clock centred;
  - a detail view with a scale gutter and time axis;
  - dotted leaders in compact rows.
- **Console fallback:** tested; no braille or emoji on `TERM=linux`.

**Left out:**
- "Rooms widen" (one wide room above two narrower ones). Equal columns
  that fill the height covered the need.
- The tmux-over-SSH check on the Pi is still to do on the real Pi.
- CPU on the Mac (release build, a light on, 15 s): ~3.4% of a core.

The original plan follows.

The goal is a dashboard that looks like one designed instrument panel, the
way btop does. Four strands:

### Graphs and meters

- **Braille area graphs.** Replace line charts and block sparklines with
  btop-style filled braille graphs. Each terminal cell is 2×4 dots, so the
  resolution is high and the fill runs from the baseline up.
- **Vertical colour gradients.** Graph colour follows the row, not the
  series: cool teal near the baseline rising to orange at the top. For
  temperature the scale is absolute, so 18° and 24° are always the same
  colours.
- **Meters.** Battery, humidity, heating demand and setpoint position become
  btop-style bars: a filled gradient section over a faint dotted track
  (`■■■■■■····`), with the value right-aligned after them.
- **Graph reuse.** One `graph` widget (any size, with optional min/max
  labels and a "now" edge) used by tiles, room panels and the detail view.
  It is snapshot-tested.

### Panels that fill the screen

- **Columns fill the full height.** Rooms in a column share it in proportion
  to their content, so there are no ragged bottoms or empty space. Space
  left in the terminal goes to graphs.
- **Flexible tiles.** Tiles get a minimum height (their readouts) plus a
  flexible graph area that absorbs spare rows. A tall terminal shows tall
  graphs; a short one falls back to today's compact rows.
- **Rooms fill the width.** Room width can grow too, e.g. one wide room on
  top with two below, chosen by total content, while keeping declaration
  order.
- **Snapshot tests** at 80×24, 120×40, 200×60 and a tall-narrow tmux pane
  check that every cell is used and nothing overlaps.

### One palette: teal structure, orange data

- **Two colour families.** Teal and blue for structure: borders, titles,
  labels, axes and graph lows. Orange and amber for data and emphasis:
  values, highlights, graph highs, lights on, heating. Red is kept for
  errors and hot temperatures only.
- **Room colours (decided).** The per-room rainbow accents go. Each room
  gets a subtle tint within the teal/blue family (teal, cyan, sea green,
  steel blue, …) for its border and title. Data colours (orange) stay the
  same in every room. `accent` names map to these tints.
- **Background (decided).** Keep the terminal's background, which is black
  on both machines. The palette is tuned for a black background, and the app
  paints no background of its own.
- **Fallback.** The 16-colour and `TERM=linux` fallbacks are kept, and
  mapped to cyan and yellow.

### Chrome and alignment

- **btop-style titles.** Titles sit inside the border (`╭─┤ Living room ├─`),
  with a small number (¹ ² ³) for jumping straight to a room with `1`–`9`.
  The clock is centred in the top border, and status shows on the right
  (`5/6 live · 💡 2`).
- **A strict label/value grid** inside tiles:
  - dim labels on the left, bold values right-aligned with units in a fixed
    column, and dotted leaders between where it helps
    (`Humidity ······ 55%`);
  - fixed-width number formatting so values don't jitter as they change;
  - consistent one-cell padding in every panel.
- **Emoji (decided for now).** Keep the current emoji, in titles and next to
  values, and judge once the rest of the refresh is in. They can be trimmed
  later if they fight the btop look.
- **Detail view.** It gets the same treatment: one large gradient graph per
  metric, stacked btop-style, with the min/avg/max grid aligned underneath.

### Order of work

1. Palette tokens and background (everything else builds on them).
2. Graph and meter widgets, with snapshots.
3. Layout v2: full-height columns and flexible tiles.
4. Chrome: titles in borders, room numbers, clock.
5. Alignment pass over every tile, the detail view and the ticker.
6. Check the result in tmux over SSH on the Pi, at 80×24 and full screen.

### Decisions (2026-11-15)

- Keep the terminal background (black).
- Keep the emoji for now; revisit after the refresh.
- Subtle teal-family tint per room.

---

## 13. Milestone 8: btop fidelity

**Result (2026-11-16):** built as specified below.
- Square boxes with `┐¹title┌` titles.
- A box per device, dimmer than its room's box.
- `Label: ── value` rows, `■` meters, and braille everywhere: device
  graphs, mini graphs in row mode, and the detail view's on/off and heating
  history.
- Featured rooms in btop's CPU-box style, with double-width columns so the
  graph has room.
- The log is in a box, and the events ticker runs along the bottom rule.
- No emoji and no big text.
- Ordinary ticker events take their room's colour.

Milestone 7 was "inspired by" btop. This one matches the reference screenshot
of btop's default theme.

**Decisions (2026-11-16):**
- **No emoji, and no big text.** The `icon` config key is accepted but
  ignored; `check` says it can be removed.
- **Everything has a panel.**
  - Every device is its own box inside its room's box.
  - The log panel is a box.
  - The top and bottom bars are rules with bracketed titles.
- **Braille for every graph and chart,** including one-row mini graphs next
  to values (like btop's per-core rows).
- **Featured devices, laid out like btop's CPU box:**
  - The featured device's graph fills the room.
  - The room's other devices stack as boxes in a column on the right, like
    btop's per-core panel.
  - In a narrow room, it falls back to plain boxes.

**Visual language, taken from the screenshot:**
- **Boxes:**
  - square corners (`┌┐└┘`);
  - titles cut into the border as `┐¹name┌`, with the number in the
    highlight colour;
  - one distinct colour per room box, cycling green, orange, magenta, red,
    blue, cyan, olive;
  - device boxes are a dimmer shade of their room's colour.
- **Rows:** bold white labels, a `─` leader in dark grey, then the value
  right-aligned. Values are coloured by what they mean.
- **Meters:** `■` cells on a grey `■` track, filled with a green → yellow
  → red gradient (or a blue scale for humidity).
- **Graphs:** braille, coloured by height (green → yellow → red), with
  temperature coloured by absolute value.
- **Selection:** the selected device's box gets a bright border, and its
  title is shown in reverse (btop's selected-row colours).
- **Compact mode** (when space is tight): devices become one-line rows
  inside the room box, like btop's core list. Each row is a label, a mini
  braille graph, and a value.

---

## 14. Milestone 9: polish and persistence

**Requests (2026-11-16):**
1. **A few icons, never emoji.** `ui/icons.rs` maps names to symbols such
   as `☼ ≈ ≋ ⌂ ☾ ★ ▮ ◉`, every one `Emoji=No`. A test embeds Unicode's
   BMP emoji ranges and fails on any icon, or any other UI symbol, that
   could render as emoji. Icons appear in room and device titles and in
   room summaries. `icon` in the config accepts names only.
2. **History that persists.**
   - One JSONL file per device under `$XDG_CACHE_HOME/mattertui/history/`.
   - One line per completed minute per metric
     (`{"m":"temperature","t":…,"lo":…,"hi":…,"v":…}`), appended as minutes
     complete. Minutes still in progress are saved on quit.
   - At startup each file is trimmed to `history_hours` and rewritten.
   - JSONL via `serde_json` (already a dependency). No compact binary
     format was in the tree, and at about 200 KB per device per day it
     isn't needed.
   - Gaps are interpolated when drawn: measurements as straight lines,
     states and setpoints as steps.
3. **Floating boxes in featured rooms.** The featured graph spans the whole
   room, and the other devices float over its top-right at their natural
   height, like btop's per-core box. They don't stretch.
4. **Subtle gradients.** Each graph blends between two shades of one hue
   by row height:
   - lavender for temperature
   - blue for humidity
   - amber for power
   - orange for heating
   - teal for battery

   Scale labels sit at the top left and bottom left.
5. **White, teal and orange everywhere.**
   - Values and labels are white.
   - Structure and secondary text are teal, in shades; room boxes are
     shades of teal.
   - Highlights, selection, lights and heating are orange.
   - Errors are a red-orange.
   - Graphs and meters use the complementary palettes in item 4.
6. **The setpoint as a thin line.** Above the data it's drawn as orange
   braille dots. Through the fill it's carved as a one-dot gap, so it
   reads as one continuous line.
