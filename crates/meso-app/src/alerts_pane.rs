/*
 * Watches/Warnings alerts pane.
 *
 * Displays active NWS alerts fetched from api.weather.gov.
 * - State/area selector (defaults to location's state, or "US" for all)
 * - Color-coded event list with severity
 * - Detail panel: fetches full description + instruction from individual alert URL
 * - Auto-refresh every 2 minutes
 */

use glib::markup_escape_text;
use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, DropDown, Label, ListBox, ListBoxRow, Orientation, PolicyType,
    ScrolledWindow, Separator, StringList, TextView, WrapMode,
};

use std::cell::RefCell;
use std::rc::Rc;

use meso_data::alerts::{self, Warning};

use crate::config::Config;
use crate::runtime;
use crate::ui::enable_status_copy;

// ── State ─────────────────────────────────────────────────────────────────────

struct AlertsState {
    warnings: Vec<Warning>,
    area: String,
}

// ── US states for dropdown ────────────────────────────────────────────────────

const AREAS: &[(&str, &str)] = &[
    ("US", "All US"),
    ("AL", "Alabama"),
    ("AK", "Alaska"),
    ("AZ", "Arizona"),
    ("AR", "Arkansas"),
    ("CA", "California"),
    ("CO", "Colorado"),
    ("CT", "Connecticut"),
    ("DE", "Delaware"),
    ("FL", "Florida"),
    ("GA", "Georgia"),
    ("HI", "Hawaii"),
    ("ID", "Idaho"),
    ("IL", "Illinois"),
    ("IN", "Indiana"),
    ("IA", "Iowa"),
    ("KS", "Kansas"),
    ("KY", "Kentucky"),
    ("LA", "Louisiana"),
    ("ME", "Maine"),
    ("MD", "Maryland"),
    ("MA", "Massachusetts"),
    ("MI", "Michigan"),
    ("MN", "Minnesota"),
    ("MS", "Mississippi"),
    ("MO", "Missouri"),
    ("MT", "Montana"),
    ("NE", "Nebraska"),
    ("NV", "Nevada"),
    ("NH", "New Hampshire"),
    ("NJ", "New Jersey"),
    ("NM", "New Mexico"),
    ("NY", "New York"),
    ("NC", "North Carolina"),
    ("ND", "North Dakota"),
    ("OH", "Ohio"),
    ("OK", "Oklahoma"),
    ("OR", "Oregon"),
    ("PA", "Pennsylvania"),
    ("RI", "Rhode Island"),
    ("SC", "South Carolina"),
    ("SD", "South Dakota"),
    ("TN", "Tennessee"),
    ("TX", "Texas"),
    ("UT", "Utah"),
    ("VT", "Vermont"),
    ("VA", "Virginia"),
    ("WA", "Washington"),
    ("WV", "West Virginia"),
    ("WI", "Wisconsin"),
    ("WY", "Wyoming"),
    ("DC", "Washington D.C."),
    ("PR", "Puerto Rico"),
    ("GU", "Guam"),
];

// ── Public builder ────────────────────────────────────────────────────────────

pub fn build_alerts_pane(shared_config: Rc<RefCell<Config>>) -> GBox {
    let vbox = GBox::new(Orientation::Vertical, 0);

    // ── Toolbar ───────────────────────────────────────────────────────────────
    let toolbar = GBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);

    let area_displays: Vec<String> = AREAS
        .iter()
        .map(|(code, label)| format!("{code} — {label}"))
        .collect();
    let area_ids: Rc<Vec<&'static str>> = Rc::new(AREAS.iter().map(|(code, _)| *code).collect());
    let area_model = StringList::new(&area_displays.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    let area_combo = DropDown::new(Some(area_model), gtk4::Expression::NONE);

    // Default to config location state
    let default_area = {
        let cfg = shared_config.borrow();
        state_from_lat_lon(cfg.location_lat, cfg.location_lon)
    };
    if let Some(pos) = area_ids.iter().position(|&id| id == default_area) {
        area_combo.set_selected(pos as u32);
    }

    let refresh_btn = Button::with_label("⟳ Refresh");
    let status = Label::new(Some("Loading alerts..."));
    status.set_hexpand(true);
    status.set_halign(gtk4::Align::Start);
    status.set_margin_start(8);
    enable_status_copy(&status);

    toolbar.append(&area_combo);
    toolbar.append(&refresh_btn);
    toolbar.append(&status);
    vbox.append(&toolbar);

    // ── Paned: list left, detail right ───────────────────────────────────────
    let paned = gtk4::Paned::new(Orientation::Horizontal);
    paned.set_vexpand(true);
    let saved_pos = shared_config.borrow().alerts_pane_position;
    if saved_pos > 0 {
        paned.set_position(saved_pos);
    } else {
        paned.set_position(320);
    }
    {
        let cfg = Rc::clone(&shared_config);
        paned.connect_position_notify(move |p| {
            cfg.borrow_mut().alerts_pane_position = p.position();
        });
    }

    // Alert list
    let list_box = ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    let list_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .child(&list_box)
        .build();
    list_scroll.set_vexpand(true);
    list_scroll.set_hexpand(false);
    paned.set_start_child(Some(&list_scroll));

    // Detail panel
    let detail_vbox = GBox::new(Orientation::Vertical, 4);
    detail_vbox.set_margin_start(4);
    detail_vbox.set_margin_end(4);
    detail_vbox.set_margin_top(4);
    detail_vbox.set_margin_bottom(4);

    let detail_header = Label::new(None);
    detail_header.set_wrap(true);
    detail_header.set_halign(gtk4::Align::Start);
    detail_header.set_selectable(true);
    detail_vbox.append(&detail_header);
    detail_vbox.append(&Separator::new(Orientation::Horizontal));

    let detail_tv = TextView::new();
    detail_tv.set_editable(false);
    detail_tv.set_wrap_mode(WrapMode::Word);
    detail_tv.set_monospace(true);
    detail_tv.set_left_margin(6);
    detail_tv.set_right_margin(6);
    detail_tv.set_top_margin(6);
    detail_tv.set_bottom_margin(6);
    let detail_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .child(&detail_tv)
        .build();
    detail_scroll.set_vexpand(true);
    detail_vbox.append(&detail_scroll);
    paned.set_end_child(Some(&detail_vbox));

    vbox.append(&paned);

    // ── Shared state ──────────────────────────────────────────────────────────
    let state = Rc::new(RefCell::new(AlertsState {
        warnings: Vec::new(),
        area: default_area.clone(),
    }));

    // ── Initial load ──────────────────────────────────────────────────────────
    load_alerts(
        default_area.clone(),
        Rc::clone(&state),
        list_box.clone(),
        status.clone(),
        detail_header.clone(),
        detail_tv.clone(),
    );

    // ── Refresh button ────────────────────────────────────────────────────────
    {
        let state_c = Rc::clone(&state);
        let lb = list_box.clone();
        let st = status.clone();
        let dh = detail_header.clone();
        let dt = detail_tv.clone();
        refresh_btn.connect_clicked(move |_| {
            let area = state_c.borrow().area.clone();
            load_alerts(
                area,
                Rc::clone(&state_c),
                lb.clone(),
                st.clone(),
                dh.clone(),
                dt.clone(),
            );
        });
    }

    // ── Area combo change ─────────────────────────────────────────────────────
    {
        let state_c = Rc::clone(&state);
        let lb = list_box.clone();
        let st = status.clone();
        let dh = detail_header.clone();
        let dt = detail_tv.clone();
        let area_ids_c = Rc::clone(&area_ids);
        area_combo.connect_selected_notify(move |combo| {
            let area = area_ids_c
                .get(combo.selected() as usize)
                .copied()
                .unwrap_or("US")
                .to_string();
            state_c.borrow_mut().area = area.clone();
            // Clear detail when switching area
            dh.set_markup("");
            dt.buffer().set_text("");
            load_alerts(
                area,
                Rc::clone(&state_c),
                lb.clone(),
                st.clone(),
                dh.clone(),
                dt.clone(),
            );
        });
    }

    // ── Auto-refresh every 2 minutes ──────────────────────────────────────────
    {
        let state_c = Rc::clone(&state);
        let lb = list_box.clone();
        let st = status.clone();
        let dh = detail_header.clone();
        let dt = detail_tv.clone();
        glib::timeout_add_seconds_local(120, move || {
            let area = state_c.borrow().area.clone();
            load_alerts(
                area,
                Rc::clone(&state_c),
                lb.clone(),
                st.clone(),
                dh.clone(),
                dt.clone(),
            );
            glib::ControlFlow::Continue
        });
    }

    // ── Row selection: fetch detail ───────────────────────────────────────────
    {
        let state_c = Rc::clone(&state);
        let dh = detail_header.clone();
        let dt = detail_tv.clone();
        list_box.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let idx = row.index() as usize;
                let (url, event, area, effective, expires, sender) = {
                    let s = state_c.borrow();
                    if let Some(w) = s.warnings.get(idx) {
                        (
                            w.url.clone(),
                            w.event.clone(),
                            w.area.clone(),
                            w.effective.clone(),
                            w.expires.clone(),
                            w.sender.clone(),
                        )
                    } else {
                        return;
                    }
                };

                // Show basic header immediately
                dh.set_markup(&format!(
                    "<b>{}</b>\n<small>{}\nEffective: {}  Expires: {}\n{}</small>",
                    markup_escape_text(&event),
                    markup_escape_text(&area),
                    markup_escape_text(&effective),
                    markup_escape_text(&expires),
                    markup_escape_text(&sender),
                ));
                dt.buffer().set_text("Fetching details...");

                let dt_c = dt.clone();
                runtime::spawn(
                    async move {
                        let client = meso_data::http::wx_client();
                        alerts::fetch_alert_detail(&client, &url).await
                    },
                    move |result| match result {
                        Ok(detail) => {
                            let mut text = String::new();
                            if !detail.headline.is_empty() {
                                text.push_str(&detail.headline);
                                text.push_str("\n\n");
                            }
                            if !detail.description.is_empty() {
                                text.push_str(&detail.description);
                            }
                            if !detail.instruction.is_empty() {
                                text.push_str("\n\nINSTRUCTIONS:\n");
                                text.push_str(&detail.instruction);
                            }
                            dt_c.buffer().set_text(&text);
                        }
                        Err(e) => {
                            dt_c.buffer()
                                .set_text(&format!("Error fetching detail:\n{e}"));
                        }
                    },
                );
            }
        });
    }

    vbox
}

// ── Data loading ──────────────────────────────────────────────────────────────

fn load_alerts(
    area: String,
    state: Rc<RefCell<AlertsState>>,
    list_box: ListBox,
    status: Label,
    detail_header: Label,
    detail_tv: TextView,
) {
    status.set_text("Fetching alerts...");
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            alerts::fetch_active_alerts(&client, &area).await
        },
        move |result| match result {
            Ok(warnings) => {
                let active: Vec<Warning> = warnings.into_iter().filter(|w| w.is_current).collect();

                if active.is_empty() {
                    status.set_text("No active alerts");
                    let row = ListBoxRow::new();
                    let lbl = Label::new(Some("No active watches or warnings"));
                    lbl.set_margin_top(8);
                    lbl.set_margin_bottom(8);
                    row.set_child(Some(&lbl));
                    list_box.append(&row);
                    detail_header.set_markup("");
                    detail_tv.buffer().set_text("");
                } else {
                    status.set_text(&format!("{} active alert(s)", active.len()));
                    for warning in &active {
                        list_box.append(&build_warning_row(warning));
                    }
                }

                state.borrow_mut().warnings = active;
            }
            Err(e) => {
                status.set_text(&format!("Error: {e}"));
            }
        },
    );
}

// ── Row builder ───────────────────────────────────────────────────────────────

fn build_warning_row(warning: &Warning) -> ListBoxRow {
    let row = ListBoxRow::new();
    let hbox = GBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);

    // Color swatch
    let (r, g, b) = meso_render::overlay::warning_color(&warning.event);
    let color_lbl = Label::new(None);
    color_lbl.set_markup(&format!(
        "<span foreground='#{r:02X}{g:02X}{b:02X}' size='large'>■</span>"
    ));
    hbox.append(&color_lbl);

    // Event + area stacked vertically
    let vb = GBox::new(Orientation::Vertical, 2);
    vb.set_hexpand(true);

    let event_lbl = Label::new(None);
    event_lbl.set_markup(&format!("<b>{}</b>", markup_escape_text(&warning.event)));
    event_lbl.set_halign(gtk4::Align::Start);
    vb.append(&event_lbl);

    let area_lbl = Label::new(Some(&warning.area));
    area_lbl.set_halign(gtk4::Align::Start);
    area_lbl.add_css_class("caption");
    area_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    vb.append(&area_lbl);

    hbox.append(&vb);

    // Expires
    let exp_lbl = Label::new(Some(&warning.expires));
    exp_lbl.set_halign(gtk4::Align::End);
    exp_lbl.add_css_class("caption");
    hbox.append(&exp_lbl);

    row.set_child(Some(&hbox));
    row
}

// ── Geo helpers ───────────────────────────────────────────────────────────────

/// Crude but fast lat/lon → US state code lookup via bounding boxes.
/// Falls back to "US" if outside all known boxes.
fn state_from_lat_lon(lat: f64, lon: f64) -> String {
    // Very rough bounding boxes for quick default; not exhaustive.
    #[rustfmt::skip]
    let boxes: &[(&str, f64, f64, f64, f64)] = &[
        // (state, lat_min, lat_max, lon_min, lon_max)
        ("AK",  51.0,  71.5, -179.0, -130.0),
        ("HI",  18.9,  22.3, -160.2, -154.8),
        ("ME",  43.0,  47.5,  -71.1,  -67.0),
        ("NH",  42.7,  45.3,  -72.6,  -70.6),
        ("VT",  42.7,  45.0,  -73.5,  -71.5),
        ("MA",  41.2,  42.9,  -73.5,  -69.9),
        ("RI",  41.1,  42.0,  -71.9,  -71.1),
        ("CT",  40.9,  42.1,  -73.7,  -71.8),
        ("NY",  40.5,  45.0,  -79.8,  -71.9),
        ("NJ",  38.9,  41.4,  -75.6,  -73.9),
        ("PA",  39.7,  42.3,  -80.5,  -74.7),
        ("DE",  38.5,  39.8,  -75.8,  -75.0),
        ("MD",  37.9,  39.7,  -79.5,  -75.0),
        ("VA",  36.5,  39.5,  -83.7,  -75.2),
        ("WV",  37.2,  40.6,  -82.6,  -77.7),
        ("NC",  33.8,  36.6,  -84.3,  -75.5),
        ("SC",  32.0,  35.2,  -83.4,  -78.5),
        ("GA",  30.4,  35.0,  -85.6,  -80.8),
        ("FL",  24.5,  31.0,  -87.6,  -80.0),
        ("AL",  30.2,  35.0,  -88.5,  -84.9),
        ("MS",  30.2,  35.0,  -91.7,  -88.1),
        ("TN",  34.9,  36.7,  -90.3,  -81.6),
        ("KY",  36.5,  39.1,  -89.6,  -81.9),
        ("OH",  38.4,  42.0,  -84.8,  -80.5),
        ("IN",  37.8,  41.8,  -88.1,  -84.8),
        ("MI",  41.7,  48.3,  -90.4,  -82.4),
        ("WI",  42.5,  47.1,  -92.9,  -86.2),
        ("MN",  43.5,  49.4,  -97.2,  -89.5),
        ("IA",  40.4,  43.5,  -96.6,  -90.1),
        ("MO",  36.0,  40.6,  -95.8,  -89.1),
        ("AR",  33.0,  36.5,  -94.6,  -89.6),
        ("LA",  28.9,  33.0,  -94.1,  -89.0),
        ("IL",  36.9,  42.5,  -91.5,  -87.5),
        ("KS",  37.0,  40.0,  -102.1, -94.6),
        ("NE",  40.0,  43.0,  -104.1, -95.3),
        ("SD",  42.5,  45.9,  -104.1, -96.4),
        ("ND",  45.9,  49.0,  -104.1, -96.6),
        ("TX",  25.8,  36.5,  -106.6, -93.5),
        ("OK",  33.6,  37.0,  -103.0, -94.4),
        ("NM",  31.3,  37.0,  -109.0, -103.0),
        ("CO",  37.0,  41.0,  -109.1, -102.0),
        ("WY",  41.0,  45.0,  -111.1, -104.1),
        ("MT",  44.4,  49.0,  -116.1, -104.1),
        ("ID",  42.0,  49.0,  -117.2, -111.1),
        ("UT",  37.0,  42.0,  -114.1, -109.0),
        ("AZ",  31.3,  37.0,  -114.8, -109.0),
        ("NV",  35.0,  42.0,  -120.0, -114.0),
        ("CA",  32.5,  42.0,  -124.5, -114.1),
        ("OR",  42.0,  46.3,  -124.6, -116.5),
        ("WA",  45.5,  49.0,  -124.8, -116.9),
    ];

    for (state, lat_min, lat_max, lon_min, lon_max) in boxes {
        if lat >= *lat_min && lat <= *lat_max && lon >= *lon_min && lon <= *lon_max {
            return state.to_string();
        }
    }
    "US".to_string()
}
