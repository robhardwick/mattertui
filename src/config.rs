//! Declarative TOML config: rooms, devices and UI options. No coordinates;
//! the layout engine places things from declaration order.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use toml::Spanned;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub ui: Ui,
    #[serde(default)]
    pub log: Log,
    #[serde(default, rename = "room")]
    pub rooms: Vec<Room>,
    #[serde(default, rename = "device")]
    pub devices: Vec<Device>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Ui {
    pub temperature_unit: TemperatureUnit,
    pub motion: bool,
    pub history_hours: u32,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            temperature_unit: TemperatureUnit::Celsius,
            motion: true,
            history_hours: 24,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemperatureUnit {
    Celsius,
    Fahrenheit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Log {
    pub panel: LogPanel,
    pub height: u16,
    pub filter: String,
}

impl Default for Log {
    fn default() -> Self {
        Self {
            panel: LogPanel::Hidden,
            height: 8,
            filter: "info,mdns_sd=warn".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogPanel {
    Hidden,
    Open,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Room {
    pub id: Spanned<String>,
    pub name: String,
    pub icon: Option<Spanned<String>>,
    pub accent: Option<Spanned<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    /// Stable key; also the commissioning label.
    pub id: Spanned<String>,
    pub name: String,
    pub room: Spanned<String>,
    pub kind: DeviceKind,
    pub icon: Option<Spanned<String>>,
    #[serde(default)]
    pub featured: bool,
    /// Override endpoint auto-detection.
    pub endpoint: Option<u16>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Plug,
    Thermostat,
    TemperatureSensor,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| {
            format!(
                "reading config {}\n(see config.example.toml in the mattertui repo for a starting point)",
                path.display()
            )
        })?;
        Self::parse(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        // toml's error Display already includes line, column and a snippet.
        let config: Config = toml::from_str(text).map_err(|e| anyhow::anyhow!("{e}"))?;
        config.validate(text)?;
        Ok(config)
    }

    fn validate(&self, text: &str) -> Result<()> {
        let mut errors = String::new();
        let mut err = |span: std::ops::Range<usize>, msg: String| {
            let (line, col) = line_col(text, span.start);
            let _ = writeln!(errors, "  line {line}, column {col}: {msg}");
        };

        if self.ui.history_hours == 0 || self.ui.history_hours > 24 * 7 {
            err(0..0, "ui.history_hours must be between 1 and 168".into());
        }

        let mut room_ids = HashSet::new();
        for room in &self.rooms {
            if let Some(msg) = invalid_id(room.id.get_ref()) {
                err(room.id.span(), format!("room id {msg}"));
            }
            if !room_ids.insert(room.id.get_ref().as_str()) {
                err(room.id.span(), format!("duplicate room id `{}`", room.id.get_ref()));
            }
            if let Some(icon) = &room.icon {
                check_icon(icon, &mut err);
            }
            if let Some(accent) = &room.accent
                && !crate::ui::theme::accent_names().any(|a| a == accent.get_ref())
            {
                err(
                    accent.span(),
                    format!(
                        "unknown accent `{}` (one of: {})",
                        accent.get_ref(),
                        crate::ui::theme::accent_names().collect::<Vec<_>>().join(", ")
                    ),
                );
            }
        }

        let mut device_ids = HashSet::new();
        for dev in &self.devices {
            if let Some(msg) = invalid_id(dev.id.get_ref()) {
                err(dev.id.span(), format!("device id {msg}"));
            }
            if !device_ids.insert(dev.id.get_ref().as_str()) {
                err(dev.id.span(), format!("duplicate device id `{}`", dev.id.get_ref()));
            }
            if !room_ids.contains(dev.room.get_ref().as_str()) {
                err(
                    dev.room.span(),
                    format!("unknown room `{}` (declare it with [[room]])", dev.room.get_ref()),
                );
            }
            if let Some(icon) = &dev.icon {
                check_icon(icon, &mut err);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            bail!("config has errors:\n{errors}")
        }
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.id.get_ref() == id)
    }
}

fn check_icon(icon: &Spanned<String>, err: &mut impl FnMut(std::ops::Range<usize>, String)) {
    if !crate::ui::icons::is_valid(icon.get_ref()) {
        err(
            icon.span(),
            format!(
                "unknown icon `{}`: use a single symbol like \"★\", or one of: {}",
                icon.get_ref(),
                crate::ui::icons::names().collect::<Vec<_>>().join(", ")
            ),
        );
    }
}

fn invalid_id(id: &str) -> Option<String> {
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    (!ok).then(|| format!("`{id}` must use only a-z, 0-9 and -"))
}

/// 1-based line and column of a byte offset.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rfind('\n').map_or(before.len(), |nl| before.len() - nl - 1) + 1;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
[ui]
temperature_unit = "fahrenheit"

[log]
panel = "open"

[[room]]
id = "living-room"
name = "Living room"

[[device]]
id = "living-room-lamp"
name = "Lamp"
room = "living-room"
kind = "plug"
"#;

    #[test]
    fn parses_good_config_with_defaults() {
        let c = Config::parse(GOOD).unwrap();
        assert_eq!(c.ui.temperature_unit, TemperatureUnit::Fahrenheit);
        assert!(c.ui.motion);
        assert_eq!(c.ui.history_hours, 24);
        assert_eq!(c.log.panel, LogPanel::Open);
        assert_eq!(c.log.height, 8);
        assert_eq!(c.devices[0].kind, DeviceKind::Plug);
        assert!(c.device("living-room-lamp").is_some());
    }

    #[test]
    fn empty_config_is_valid() {
        let c = Config::parse("").unwrap();
        assert!(c.rooms.is_empty() && c.devices.is_empty());
    }

    #[test]
    fn reports_unknown_room_with_position() {
        let text = GOOD.replace("room = \"living-room\"", "room = \"kitchen\"");
        let e = format!("{:#}", Config::parse(&text).unwrap_err());
        assert!(e.contains("unknown room `kitchen`"), "{e}");
        assert!(e.contains("line 15"), "{e}");
    }

    #[test]
    fn reports_duplicates_and_bad_ids() {
        let text = format!("{GOOD}\n[[device]]\nid = \"living-room-lamp\"\nname = \"x\"\nroom = \"living-room\"\nkind = \"plug\"\n[[room]]\nid = \"Bad Id\"\nname = \"x\"\n");
        let e = format!("{:#}", Config::parse(&text).unwrap_err());
        assert!(e.contains("duplicate device id"), "{e}");
        assert!(e.contains("`Bad Id` must use only"), "{e}");
    }

    #[test]
    fn rejects_unknown_kind_and_fields() {
        assert!(Config::parse(&GOOD.replace("\"plug\"", "\"toaster\"")).is_err());
        assert!(Config::parse("[ui]\ncolour = 1\n").is_err());
    }
}
