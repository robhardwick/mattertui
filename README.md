# mattertui

> [!WARNING]
> Vibe coded

A read-only terminal dashboard showing live graphs of Matter devices, grouped by room.

## Install

```bash
cargo install --git https://github.com/robhardwick/mattertui
mattertui fetch-roots   # download Matter attestation roots, needed for pairing
```

### Configure

```bash
mkdir -p ~/.config/mattertui
curl -o ~/.config/mattertui/config.toml \
  https://raw.githubusercontent.com/robhardwick/mattertui/main/config.example.toml
$EDITOR ~/.config/mattertui/config.toml   # add your rooms and devices
```

### Pair

For each device, get a code from the Home app (device → Settings → Turn On Pairing Mode), then:

```bash
mattertui pair living-room-lamp 3497-011-2332
mattertui check   # confirm the config is valid and see which devices are paired
```

## Usage

### List paired devices

```bash
mattertui nodes --probe
```

### Show the dashboard

```bash
mattertui
```

### Unpair a device

```bash
mattertui unpair living-room-lamp
```

### Use a different config file

```bash
mattertui --config ~/my-config.toml
```
