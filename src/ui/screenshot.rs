//! Renders a made-up house to HTML for the README screenshot. Never reads
//! the real config, so no real names, rooms or devices leak into it.
//!
//! ```sh
//! SCREENSHOT=/tmp/screenshot.html cargo test readme_screenshot -- --ignored
//! "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new \
//!   --hide-scrollbars --force-device-scale-factor=2 --window-size=1628,882 \
//!   --screenshot="$PWD/docs/screenshot.png" file:///tmp/screenshot.html
//! ```
//!
//! The window size is 190×50 cells of 8.4×17px, plus 16px padding each side.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use super::app::App;
use super::theme::{Depth, Theme};
use crate::config::Config;
use crate::logbuf::LogBuffer;
use crate::state::{DeviceInfo, History, Metric, Status, ThermostatInfo, Transport, unix_minute};

const WIDTH: u16 = 190;
const HEIGHT: u16 = 50;
const BACKGROUND: &str = "#141414";
const FOREGROUND: &str = "#dedede";

const CONFIG: &str = r#"
[[room]]
id = "living-room"
name = "Living room"
icon = "sofa"

[[room]]
id = "kitchen"
name = "Kitchen"
icon = "kitchen"

[[room]]
id = "bedroom"
name = "Bedroom"
icon = "bed"

[[room]]
id = "office"
name = "Office"
icon = "desk"

[[device]]
id = "heating"
name = "Heating"
room = "living-room"
kind = "thermostat"
featured = true

[[device]]
id = "floor-lamp"
name = "Floor lamp"
room = "living-room"
kind = "plug"

[[device]]
id = "sofa"
name = "Sofa sensor"
room = "living-room"
kind = "temperature_sensor"

[[device]]
id = "kitchen-temp"
name = "Worktop"
room = "kitchen"
kind = "temperature_sensor"
featured = true

[[device]]
id = "kettle"
name = "Kettle"
room = "kitchen"
kind = "plug"
icon = "plug"

[[device]]
id = "under-sink"
name = "Under sink"
room = "kitchen"
kind = "leak_sensor"

[[device]]
id = "bedside"
name = "Bedside"
room = "bedroom"
kind = "temperature_sensor"
featured = true

[[device]]
id = "office"
name = "Desk"
room = "office"
kind = "temperature_sensor"
featured = true
"#;

fn demo_app() -> App {
    let config = Config::parse(CONFIG).unwrap();
    let mut app = App::new(config, LogBuffer::new(), Theme { depth: Depth::Truecolor, ascii: false });
    let now = unix_minute();
    let info = |transport| DeviceInfo {
        node_id: 1,
        vendor: Some("Acme".into()),
        product: Some("Gadget".into()),
        firmware: Some("1.0".into()),
        transport,
        endpoint: 1,
        thermostat: None,
    };
    // An hour of history from `f(minutes ago)`.
    let set = |app: &mut App, i: usize, m: Metric, f: &dyn Fn(f64) -> f64| {
        let h = app.devices[i].history.entry(m).or_insert_with(|| History::new(1440));
        for k in (0..60).rev() {
            h.record(now - k, f(k as f64));
        }
        app.devices[i].current.insert(m, Some(f(0.0)));
    };
    for d in &mut app.devices {
        d.status = Status::Live { max_interval_s: 60 };
    }
    // Heating: warming towards the setpoint, burner cycling.
    app.devices[0].info = Some(DeviceInfo {
        thermostat: Some(ThermostatInfo {
            can_heat: true,
            can_cool: false,
            heat_limits: Some((5.0, 30.0)),
            running_estimated: false,
        }),
        ..info(Transport::Thread)
    });
    set(&mut app, 0, Metric::Temperature, &|k| 21.0 - k * 0.04 + (k / 6.0).sin() * 0.15);
    set(&mut app, 0, Metric::HeatSetpoint, &|k| if k > 45.0 { 18.0 } else { 21.5 });
    set(&mut app, 0, Metric::Running, &|k| if k < 45.0 && (k as i64 / 8) % 2 == 0 { 1.0 } else { 0.0 });
    set(&mut app, 0, Metric::Mode, &|_| 4.0);
    set(&mut app, 0, Metric::Humidity, &|k| 48.0 + (k / 15.0).cos() * 2.0);
    app.devices[0].current.insert(Metric::BatteryLevel, Some(0.0));
    // Floor lamp: on for the last 20 minutes.
    app.devices[1].info = Some(info(Transport::WiFi));
    set(&mut app, 1, Metric::On, &|k| if k < 20.0 { 1.0 } else { 0.0 });
    // Sofa sensor.
    app.devices[2].info = Some(info(Transport::Thread));
    set(&mut app, 2, Metric::Temperature, &|k| 20.4 - k * 0.02 + (k / 9.0).sin() * 0.3);
    set(&mut app, 2, Metric::Humidity, &|_| 51.0);
    set(&mut app, 2, Metric::Battery, &|_| 82.0);
    // Kitchen: cooking warmed it up.
    app.devices[3].info = Some(info(Transport::Thread));
    set(&mut app, 3, Metric::Temperature, &|k| 19.5 + 2.5 * (-((k - 20.0) / 12.0).powi(2)).exp());
    set(&mut app, 3, Metric::Humidity, &|k| 55.0 + 10.0 * (-((k - 18.0) / 10.0).powi(2)).exp());
    set(&mut app, 3, Metric::Battery, &|_| 64.0);
    // Kettle: boiled twice.
    app.devices[4].info = Some(info(Transport::WiFi));
    set(&mut app, 4, Metric::On, &|k| if (30.0..34.0).contains(&k) || (6.0..9.0).contains(&k) { 1.0 } else { 0.0 });
    // Under the sink: dry.
    app.devices[5].info = Some(info(Transport::Thread));
    set(&mut app, 5, Metric::Leak, &|_| 0.0);
    set(&mut app, 5, Metric::Battery, &|_| 100.0);
    // Bedroom: slowly cooling.
    app.devices[6].info = Some(info(Transport::Thread));
    set(&mut app, 6, Metric::Temperature, &|k| 17.4 + k * 0.012 + (k / 7.0).sin() * 0.08);
    set(&mut app, 6, Metric::Humidity, &|_| 57.0);
    set(&mut app, 6, Metric::Battery, &|_| 45.0);
    // Office: sun coming through the window.
    app.devices[7].info = Some(info(Transport::Thread));
    set(&mut app, 7, Metric::Temperature, &|k| 22.6 - k * 0.03 + (k / 4.0).sin() * 0.1);
    set(&mut app, 7, Metric::Humidity, &|_| 44.0);
    set(&mut app, 7, Metric::Battery, &|_| 91.0);
    app.selected = 0;
    app.motion = false;
    app
}

fn css(c: Color, default: &str) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => default.to_string(),
        Color::Black => "#000000".into(),
        Color::White => "#f2f2f2".into(),
        Color::Gray => "#8e8e8e".into(),
        Color::DarkGray => "#3c3c3c".into(),
        _ => default.to_string(),
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Braille drawn as solid dots, as terminals do, rather than in a font that
/// may also draw the unset dots.
fn braille_svg(s: &str) -> Option<String> {
    let c = s.chars().next().filter(|c| ('\u{2800}'..='\u{28FF}').contains(c))?;
    let bits = c as u32 - 0x2800;
    // Bit → (column, row), per the Unicode braille layout.
    const DOTS: [(u32, u32); 8] = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)];
    let mut rects = String::new();
    for (bit, (col, row)) in DOTS.iter().enumerate() {
        if bits & (1 << bit) != 0 {
            rects.push_str(&format!(
                "<rect x=\"{:.1}\" y=\"{:.2}\" width=\"2.6\" height=\"2.6\" rx=\"0.6\"/>",
                1.2 + f64::from(*col) * 3.6,
                0.9 + f64::from(*row) * 4.25,
            ));
        }
    }
    Some(format!("<svg width=\"8.4\" height=\"17\" fill=\"currentColor\">{rects}</svg>"))
}

/// One fixed-size cell per character, so fallback fonts (braille) can't
/// break the grid.
fn to_html(buf: &Buffer) -> String {
    let mut rows = String::new();
    for y in 0..buf.area.height {
        rows.push_str("<div class=r>");
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let mut style = String::new();
            let fg = css(cell.fg, FOREGROUND);
            if fg != FOREGROUND {
                style.push_str(&format!("color:{fg};"));
            }
            let bg = css(cell.bg, BACKGROUND);
            if bg != BACKGROUND {
                style.push_str(&format!("background:{bg};"));
            }
            if cell.modifier.contains(Modifier::BOLD) {
                style.push_str("font-weight:700;");
            }
            let sym = match cell.symbol() {
                " " => "&nbsp;".to_string(),
                s => braille_svg(s).unwrap_or_else(|| escape(s)),
            };
            if style.is_empty() {
                rows.push_str(&format!("<i>{sym}</i>"));
            } else {
                rows.push_str(&format!("<i style=\"{style}\">{sym}</i>"));
            }
        }
        rows.push_str("</div>\n");
    }
    format!(
        r#"<!doctype html>
<meta charset="utf-8">
<style>
  html, body {{ margin: 0; background: {BACKGROUND}; }}
  .t {{ padding: 16px; display: inline-block; color: {FOREGROUND};
        font: 14px/17px "SF Mono", Menlo, monospace; }}
  .r {{ display: flex; height: 17px; }}
  .r i {{ font-style: normal; width: 8.4px; height: 17px; display: inline-block;
          text-align: center; overflow: visible; white-space: pre; }}
  .r svg {{ display: block; }}
</style>
<div class=t>
{rows}</div>
"#
    )
}

#[test]
#[ignore = "writes the README screenshot; run on demand"]
fn readme_screenshot() {
    let path = std::env::var("SCREENSHOT").unwrap_or_else(|_| "docs/screenshot.html".into());
    let mut app = demo_app();
    let mut term = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    term.draw(|f| super::draw(f, &mut app)).unwrap();
    // A second frame, once the layout is known (floating boxes use it).
    term.draw(|f| super::draw(f, &mut app)).unwrap();
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&path, to_html(term.backend().buffer())).unwrap();
    eprintln!("wrote {path}");
}
