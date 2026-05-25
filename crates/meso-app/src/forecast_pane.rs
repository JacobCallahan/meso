/*
 * Forecast pane: NWS 7-day forecast + hourly breakdown + current conditions.
 *
 * Tab toggle switches between:
 *   - 7-Day view: named periods with temp, wind, PoP, detailed forecast
 *   - Hourly view: compact per-hour table grouped by day
 */

use glib::markup_escape_text;
use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, Label, Orientation, ScrolledWindow, Separator, Stack, ToggleButton,
};

use std::cell::RefCell;
use std::rc::Rc;

use meso_data::forecast::{self, ForecastPeriod, HourlyPeriod};

use crate::config::Config;
use crate::runtime;
use crate::ui::enable_status_copy;

// ── State ─────────────────────────────────────────────────────────────────────

struct ForecastState {
    lat: f64,
    lon: f64,
}

// ── Public builder ────────────────────────────────────────────────────────────

pub fn build_forecast_pane(config: &Config) -> GBox {
    let vbox = GBox::new(Orientation::Vertical, 0);

    // ── Toolbar ───────────────────────────────────────────────────────────────
    let toolbar = GBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);

    let seven_day_btn = ToggleButton::with_label("7-Day");
    let hourly_btn = ToggleButton::with_label("Hourly");
    seven_day_btn.set_active(true);
    hourly_btn.set_group(Some(&seven_day_btn));

    let refresh_btn = Button::with_label("⟳");
    let status = Label::new(Some("Loading..."));
    status.set_hexpand(true);
    status.set_halign(gtk4::Align::Start);
    status.set_margin_start(8);
    enable_status_copy(&status);

    toolbar.append(&seven_day_btn);
    toolbar.append(&hourly_btn);
    toolbar.append(&refresh_btn);
    toolbar.append(&status);
    vbox.append(&toolbar);

    // ── Current conditions ────────────────────────────────────────────────────
    let current_box = GBox::new(Orientation::Horizontal, 12);
    current_box.add_css_class("card");
    current_box.set_margin_top(4);
    current_box.set_margin_start(8);
    current_box.set_margin_end(8);
    current_box.set_margin_bottom(4);

    let temp_label = Label::new(Some("--°F"));
    temp_label.add_css_class("title-2");
    temp_label.set_width_chars(7);
    current_box.append(&temp_label);

    let conditions_box = GBox::new(Orientation::Vertical, 2);
    let sky_label = Label::new(Some("--"));
    sky_label.set_halign(gtk4::Align::Start);
    let wind_label = Label::new(Some("Wind: --"));
    wind_label.set_halign(gtk4::Align::Start);
    let dewpoint_label = Label::new(Some("Dewpoint: --°F"));
    dewpoint_label.set_halign(gtk4::Align::Start);
    let pressure_label = Label::new(Some("Pressure: -- hPa"));
    pressure_label.set_halign(gtk4::Align::Start);
    let vis_label = Label::new(Some("Visibility: -- mi"));
    vis_label.set_halign(gtk4::Align::Start);
    conditions_box.append(&sky_label);
    conditions_box.append(&wind_label);
    conditions_box.append(&dewpoint_label);
    conditions_box.append(&pressure_label);
    conditions_box.append(&vis_label);
    current_box.append(&conditions_box);
    vbox.append(&current_box);
    vbox.append(&Separator::new(Orientation::Horizontal));

    // ── Stack: 7-day / hourly ─────────────────────────────────────────────────
    let stack = Stack::new();
    stack.set_vexpand(true);

    // 7-day tab
    let seven_scroll = ScrolledWindow::new();
    seven_scroll.set_vexpand(true);
    let seven_box = GBox::new(Orientation::Vertical, 0);
    seven_scroll.set_child(Some(&seven_box));
    stack.add_named(&seven_scroll, Some("7day"));

    // Hourly tab
    let hourly_scroll = ScrolledWindow::new();
    hourly_scroll.set_vexpand(true);
    let hourly_box = GBox::new(Orientation::Vertical, 0);
    hourly_scroll.set_child(Some(&hourly_box));
    stack.add_named(&hourly_scroll, Some("hourly"));

    vbox.append(&stack);

    // ── Tab toggle wiring ─────────────────────────────────────────────────────
    {
        let stack_c = stack.clone();
        seven_day_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                stack_c.set_visible_child_name("7day");
            }
        });
    }
    {
        let stack_c = stack.clone();
        hourly_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                stack_c.set_visible_child_name("hourly");
            }
        });
    }

    // ── State + initial load ──────────────────────────────────────────────────
    let state = Rc::new(RefCell::new(ForecastState {
        lat: config.location_lat,
        lon: config.location_lon,
    }));

    load_all(
        config.location_lat,
        config.location_lon,
        temp_label.clone(),
        sky_label.clone(),
        wind_label.clone(),
        dewpoint_label.clone(),
        pressure_label.clone(),
        vis_label.clone(),
        seven_box.clone(),
        hourly_box.clone(),
        status.clone(),
    );

    // Refresh button
    {
        let state_c = Rc::clone(&state);
        let (tl, sl, wl, dl, pl, vl) = (
            temp_label.clone(),
            sky_label.clone(),
            wind_label.clone(),
            dewpoint_label.clone(),
            pressure_label.clone(),
            vis_label.clone(),
        );
        let (sb, hb, st) = (seven_box.clone(), hourly_box.clone(), status.clone());
        refresh_btn.connect_clicked(move |_| {
            let s = state_c.borrow();
            load_all(
                s.lat,
                s.lon,
                tl.clone(),
                sl.clone(),
                wl.clone(),
                dl.clone(),
                pl.clone(),
                vl.clone(),
                sb.clone(),
                hb.clone(),
                st.clone(),
            );
        });
    }

    vbox
}

// ── Data loading ──────────────────────────────────────────────────────────────

fn load_all(
    lat: f64,
    lon: f64,
    temp_label: Label,
    sky_label: Label,
    wind_label: Label,
    dewpoint_label: Label,
    pressure_label: Label,
    vis_label: Label,
    seven_box: GBox,
    hourly_box: GBox,
    status: Label,
) {
    status.set_text("Fetching forecast...");

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            let point = forecast::resolve_point(&client, lat, lon).await?;
            let obs = forecast::fetch_observations(&client, &point.observation_stations).await;
            let periods = forecast::fetch_forecast(&client, &point.forecast).await?;
            let hourly = forecast::fetch_hourly_forecast(&client, &point.forecast_hourly).await?;
            Ok::<_, anyhow::Error>((obs, periods, hourly, point.relative_location))
        },
        move |result| {
            match result {
                Ok((obs_result, periods, hourly, rel_loc)) => {
                    // Status
                    let loc_str = rel_loc
                        .map(|l| format!(" — {}, {}", l.properties.city, l.properties.state))
                        .unwrap_or_default();
                    status.set_text(&format!(
                        "{} periods, {} hourly hours{loc_str}",
                        periods.len(),
                        hourly.len()
                    ));

                    // Current conditions
                    if let Ok(obs) = obs_result {
                        if let Some(t) = obs.temperature_f() {
                            temp_label.set_text(&format!("{:.0}°F", t));
                        }
                        sky_label.set_text(&obs.text_description);
                        match (obs.wind_speed_kts(), obs.wind_direction) {
                            (Some(spd), Some(dir)) => {
                                wind_label.set_text(&format!("Wind: {:.0}° at {:.0} kt", dir, spd))
                            }
                            (Some(spd), None) => {
                                wind_label.set_text(&format!("Wind: {:.0} kt", spd))
                            }
                            _ => {}
                        }
                        if let Some(dp) = obs.dewpoint_f() {
                            dewpoint_label.set_text(&format!("Dewpoint: {:.0}°F", dp));
                        }
                        if let Some(p) = obs.sea_level_pressure_pa {
                            pressure_label.set_text(&format!("Pressure: {:.1} hPa", p / 100.0));
                        }
                        if let Some(v) = obs.visibility_m {
                            vis_label.set_text(&format!("Visibility: {:.1} mi", v / 1609.0));
                        }
                    }

                    // 7-day rows
                    clear_box(&seven_box);
                    for period in &periods {
                        seven_box.append(&build_7day_row(period));
                        seven_box.append(&Separator::new(Orientation::Horizontal));
                    }

                    // Hourly rows
                    clear_box(&hourly_box);
                    let mut last_day = String::new();
                    for period in &hourly {
                        let day = period.day_label();
                        if day != last_day {
                            hourly_box.append(&build_day_header(&day));
                            last_day = day;
                        }
                        hourly_box.append(&build_hourly_row(period));
                    }
                }
                Err(e) => {
                    status.set_text(&format!("Error: {e}"));
                }
            }
        },
    );
}

fn clear_box(b: &GBox) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

// ── Row builders ──────────────────────────────────────────────────────────────

fn build_7day_row(period: &ForecastPeriod) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    row.set_margin_start(8);
    row.set_margin_end(8);

    let name_lbl = Label::new(Some(&period.name));
    name_lbl.set_width_chars(18);
    name_lbl.set_halign(gtk4::Align::Start);
    name_lbl.add_css_class("heading");
    row.append(&name_lbl);

    let temp_lbl = Label::new(Some(&format!(
        "{}°{}",
        period.temperature, period.temperature_unit
    )));
    temp_lbl.set_width_chars(6);
    row.append(&temp_lbl);

    let wind_lbl = Label::new(Some(&format!(
        "{} {}",
        period.wind_speed, period.wind_direction
    )));
    wind_lbl.set_width_chars(16);
    wind_lbl.set_halign(gtk4::Align::Start);
    row.append(&wind_lbl);

    let fcst_lbl = Label::new(Some(&period.short_forecast));
    fcst_lbl.set_hexpand(true);
    fcst_lbl.set_halign(gtk4::Align::Start);
    fcst_lbl.set_wrap(true);
    row.append(&fcst_lbl);

    if let Some(pop) = period.probability_of_precipitation {
        if pop > 0 {
            let pop_lbl = Label::new(None);
            pop_lbl.set_markup(&format!("<span foreground='#5af'>PoP: {}%</span>", pop));
            pop_lbl.set_halign(gtk4::Align::End);
            row.append(&pop_lbl);
        }
    }

    row
}

fn build_day_header(day: &str) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 0);
    row.add_css_class("card");
    row.set_margin_top(4);
    row.set_margin_bottom(0);
    let lbl = Label::new(None);
    lbl.set_markup(&format!("<b>{}</b>", markup_escape_text(day)));
    lbl.set_halign(gtk4::Align::Start);
    lbl.set_margin_start(8);
    lbl.set_margin_top(3);
    lbl.set_margin_bottom(3);
    row.append(&lbl);
    row
}

fn build_hourly_row(period: &HourlyPeriod) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 4);
    row.set_margin_top(2);
    row.set_margin_bottom(2);
    row.set_margin_start(12);
    row.set_margin_end(8);

    // Hour
    let hour_lbl = Label::new(Some(&period.hour_label()));
    hour_lbl.set_width_chars(7);
    hour_lbl.set_halign(gtk4::Align::End);
    row.append(&hour_lbl);

    // Temp
    let temp_lbl = Label::new(None);
    let temp_color = temp_color(period.temperature);
    temp_lbl.set_markup(&format!(
        "<span foreground='{temp_color}'><b>{}°{}</b></span>",
        period.temperature, period.temperature_unit
    ));
    temp_lbl.set_width_chars(7);
    temp_lbl.set_halign(gtk4::Align::End);
    row.append(&temp_lbl);

    // Dewpoint
    if let Some(dp_c) = period.dewpoint_c {
        let dp_f = dp_c * 9.0 / 5.0 + 32.0;
        let dp_lbl = Label::new(Some(&format!("Dp:{:.0}°", dp_f)));
        dp_lbl.set_width_chars(8);
        dp_lbl.set_halign(gtk4::Align::End);
        dp_lbl.add_css_class("dim-label");
        row.append(&dp_lbl);
    } else {
        let pad = Label::new(Some(""));
        pad.set_width_chars(8);
        row.append(&pad);
    }

    // Wind
    let wind_lbl = Label::new(Some(&format!(
        "{} {}",
        period.wind_speed, period.wind_direction
    )));
    wind_lbl.set_width_chars(16);
    wind_lbl.set_halign(gtk4::Align::Start);
    row.append(&wind_lbl);

    // PoP
    if let Some(pop) = period.probability_of_precipitation {
        if pop > 0 {
            let pop_lbl = Label::new(None);
            pop_lbl.set_markup(&format!("<span foreground='#5af'>{}%</span>", pop));
            pop_lbl.set_width_chars(5);
            pop_lbl.set_halign(gtk4::Align::End);
            row.append(&pop_lbl);
        } else {
            let pad = Label::new(Some(""));
            pad.set_width_chars(5);
            row.append(&pad);
        }
    }

    // Conditions
    let cond_lbl = Label::new(Some(&period.short_forecast));
    cond_lbl.set_hexpand(true);
    cond_lbl.set_halign(gtk4::Align::Start);
    row.append(&cond_lbl);

    row
}

/// Map temperature (°F) to a color hex string for markup.
fn temp_color(temp_f: i32) -> &'static str {
    match temp_f {
        t if t >= 100 => "#ff2020",
        t if t >= 90 => "#ff6030",
        t if t >= 80 => "#ff9030",
        t if t >= 70 => "#ffcc00",
        t if t >= 60 => "#ccff00",
        t if t >= 50 => "#88ff44",
        t if t >= 40 => "#44ffaa",
        t if t >= 32 => "#44ccff",
        t if t >= 20 => "#88aaff",
        t if t >= 0 => "#aaaaff",
        _ => "#ccccff",
    }
}
