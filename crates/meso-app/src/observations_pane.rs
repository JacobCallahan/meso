/*
 * Surface Observations pane — state-based collapsible METAR tree with
 * search, favorites, and TAF detail.
 *
 * Layout: horizontal Paned
 *   Left:  Search bar + TreeView of Favorites + states (collapsed by default)
 *          Expanding a state triggers async fetch of all METARs for that state.
 *   Right: Decoded METAR detail + TAF (if available)
 *
 * Station list is derived from the bundled obs_stations.txt via wx_data.
 * METAR data is cached for 10 minutes.  TAF data is cached for 30 minutes.
 */

use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, CellRendererText, Label, Orientation, Paned, PolicyType, ScrolledWindow,
    SearchEntry, Separator, TreeModelFilter, TreeStore, TreeView, TreeViewColumn,
};

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use meso_data::observations::{self, deg_to_compass, time_ago_str, Observation, Taf};

use crate::config::Config;
use crate::runtime;

// ── Constants ─────────────────────────────────────────────────────────────────

const COL_LABEL: u32 = 0;
const COL_ID: u32 = 1;
const COL_TYPE: u32 = 2;

const LOADING_SENTINEL: &str = "__loading__";
const ROW_FAVORITES: &str = "__favorites__";

static US_STATES: &[(&str, &str)] = &[
    ("Alabama", "AL"),
    ("Alaska", "AK"),
    ("Arizona", "AZ"),
    ("Arkansas", "AR"),
    ("California", "CA"),
    ("Colorado", "CO"),
    ("Connecticut", "CT"),
    ("Delaware", "DE"),
    ("District of Columbia", "DC"),
    ("Florida", "FL"),
    ("Georgia", "GA"),
    ("Hawaii", "HI"),
    ("Idaho", "ID"),
    ("Illinois", "IL"),
    ("Indiana", "IN"),
    ("Iowa", "IA"),
    ("Kansas", "KS"),
    ("Kentucky", "KY"),
    ("Louisiana", "LA"),
    ("Maine", "ME"),
    ("Maryland", "MD"),
    ("Massachusetts", "MA"),
    ("Michigan", "MI"),
    ("Minnesota", "MN"),
    ("Mississippi", "MS"),
    ("Missouri", "MO"),
    ("Montana", "MT"),
    ("Nebraska", "NE"),
    ("Nevada", "NV"),
    ("New Hampshire", "NH"),
    ("New Jersey", "NJ"),
    ("New Mexico", "NM"),
    ("New York", "NY"),
    ("North Carolina", "NC"),
    ("North Dakota", "ND"),
    ("Ohio", "OH"),
    ("Oklahoma", "OK"),
    ("Oregon", "OR"),
    ("Pennsylvania", "PA"),
    ("Rhode Island", "RI"),
    ("South Carolina", "SC"),
    ("South Dakota", "SD"),
    ("Tennessee", "TN"),
    ("Texas", "TX"),
    ("Utah", "UT"),
    ("Vermont", "VT"),
    ("Virginia", "VA"),
    ("Washington", "WA"),
    ("West Virginia", "WV"),
    ("Wisconsin", "WI"),
    ("Wyoming", "WY"),
];

// ── State ─────────────────────────────────────────────────────────────────────

struct ObsState {
    loaded: HashMap<String, Observation>,
    selected_station: Option<String>,
}

impl ObsState {
    fn new() -> Self {
        ObsState {
            loaded: HashMap::new(),
            selected_station: None,
        }
    }
}

// ── Widget builder ────────────────────────────────────────────────────────────

pub fn build_observations_pane(shared_cfg: Rc<RefCell<Config>>) -> GBox {
    let state = Rc::new(RefCell::new(ObsState::new()));

    let vbox = GBox::new(Orientation::Vertical, 0);

    // ── Toolbar ───────────────────────────────────────────────────────────────
    let toolbar = GBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(6);
    toolbar.set_margin_end(6);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);
    toolbar.add_css_class("toolbar");

    let title_label = Label::new(Some("Surface Observations"));
    title_label.set_hexpand(true);
    title_label.set_halign(gtk4::Align::Start);

    let status_label = Label::new(Some("Expand a state to load stations"));
    status_label.set_halign(gtk4::Align::End);
    status_label.add_css_class("dim-label");

    toolbar.append(&title_label);
    toolbar.append(&status_label);
    vbox.append(&toolbar);

    // ── Paned layout ──────────────────────────────────────────────────────────
    let paned = Paned::new(Orientation::Horizontal);
    paned.set_vexpand(true);

    // ── Left: search + tree ───────────────────────────────────────────────────
    let left_box = GBox::new(Orientation::Vertical, 0);

    let search = SearchEntry::new();
    search.set_placeholder_text(Some("Filter stations…"));
    search.set_margin_start(4);
    search.set_margin_end(4);
    search.set_margin_top(4);
    search.set_margin_bottom(4);
    left_box.append(&search);

    let store = TreeStore::new(&[glib::Type::STRING, glib::Type::STRING, glib::Type::STRING]);

    // Favorites row (top, collapsed)
    let fav_iter = store.append(None);
    store.set(
        &fav_iter,
        &[
            (COL_LABEL, &"⭐ Favorites"),
            (COL_ID, &ROW_FAVORITES),
            (COL_TYPE, &"favorites"),
        ],
    );

    // State rows
    for (name, code) in US_STATES {
        let label = format!("{name} ({code})");
        let si = store.append(None);
        store.set(
            &si,
            &[
                (COL_LABEL, &label.as_str()),
                (COL_ID, code),
                (COL_TYPE, &"state"),
            ],
        );
        let dummy = store.append(Some(&si));
        store.set(
            &dummy,
            &[
                (COL_LABEL, &"Loading…"),
                (COL_ID, &LOADING_SENTINEL),
                (COL_TYPE, &LOADING_SENTINEL),
            ],
        );
    }

    // ── Filter model wrapping the store ──────────────────────────────────────
    let filter_text: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let filter_model = TreeModelFilter::new(&store, None);
    {
        let ft = Rc::clone(&filter_text);
        filter_model.set_visible_func(move |model, iter| {
            let text = ft.borrow();
            if text.is_empty() {
                return true;
            }
            let text = text.as_str();
            let row_type: String = model.get::<String>(iter, COL_TYPE as i32);
            match row_type.as_str() {
                "favorites" | "state" => {
                    // Show parent row only if at least one child station matches.
                    // Unloaded states (first child = sentinel) are hidden during filtering.
                    let first_child = model.iter_children(Some(iter));
                    match first_child {
                        None => false,
                        Some(c) => {
                            let ct: String = model.get::<String>(&c, COL_TYPE as i32);
                            if ct == LOADING_SENTINEL {
                                return false;
                            }
                            let ci = c;
                            loop {
                                let cl: String = model.get::<String>(&ci, COL_LABEL as i32);
                                if cl.to_lowercase().contains(text) {
                                    return true;
                                }
                                if !model.iter_next(&ci) {
                                    break;
                                }
                            }
                            false
                        }
                    }
                }
                "station" => {
                    let label: String = model.get::<String>(iter, COL_LABEL as i32);
                    label.to_lowercase().contains(text)
                }
                _ => false, // hide sentinels and unknown rows during filtering
            }
        });
    }

    let tree_view = TreeView::with_model(&filter_model);
    tree_view.set_headers_visible(false);
    tree_view.set_activate_on_single_click(true);

    let col = TreeViewColumn::new();
    let cell = CellRendererText::new();
    col.pack_start(&cell, true);
    col.add_attribute(&cell, "text", COL_LABEL as i32);
    tree_view.append_column(&col);

    let tree_scroll = ScrolledWindow::new();
    tree_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    tree_scroll.set_vexpand(true);
    tree_scroll.set_child(Some(&tree_view));
    left_box.append(&tree_scroll);
    left_box.set_width_request(270);
    paned.set_start_child(Some(&left_box));

    // ── Right: detail ─────────────────────────────────────────────────────────
    let detail_scroll = ScrolledWindow::new();
    detail_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    detail_scroll.set_hexpand(true);

    let detail_box = GBox::new(Orientation::Vertical, 8);
    detail_box.set_margin_start(12);
    detail_box.set_margin_end(12);
    detail_box.set_margin_top(12);
    detail_box.set_margin_bottom(12);

    let placeholder = Label::new(Some(
        "Expand a state and select a station to view observations",
    ));
    placeholder.add_css_class("dim-label");
    placeholder.set_valign(gtk4::Align::Center);
    placeholder.set_vexpand(true);
    placeholder.set_wrap(true);
    detail_box.append(&placeholder);

    detail_scroll.set_child(Some(&detail_box));
    paned.set_end_child(Some(&detail_scroll));

    {
        let paned_c = paned.clone();
        paned.connect_realize(move |p| {
            let w = p.width();
            if w > 0 {
                paned_c.set_position((w as f64 * 0.38) as i32);
            }
        });
    }
    vbox.append(&paned);

    // ── Search filter ─────────────────────────────────────────────────────────
    {
        let filter_model_s = filter_model.clone();
        let filter_text_s = Rc::clone(&filter_text);
        let tree_c = tree_view.clone();
        search.connect_search_changed(move |s| {
            let text = s.text().to_lowercase();
            *filter_text_s.borrow_mut() = text.clone();
            filter_model_s.refilter();
            if !text.is_empty() {
                // Expand all visible state rows so matching stations are visible.
                tree_c.expand_all();
            }
        });
    }

    // ── row-expanded: lazy-load ───────────────────────────────────────────────
    {
        let store_c = store.clone();
        let state_c = Rc::clone(&state);
        let status_c = status_label.clone();
        let cfg_c = Rc::clone(&shared_cfg);
        let filter_model_r = filter_model.clone();

        tree_view.connect_row_expanded(move |tv, _iter, filter_path| {
            // Convert filter model path → store path
            let path = match filter_model_r.convert_path_to_child_path(filter_path) {
                Some(p) => p,
                None => return,
            };
            // Only act on state rows that haven't loaded yet
            let parent_iter = match store_c.iter(&path) {
                Some(i) => i,
                None => return,
            };
            let row_type: String = store_c.get::<String>(&parent_iter, COL_TYPE as i32);
            if row_type != "state" {
                return;
            }

            let mut cp = path.clone();
            cp.append_index(0);
            let child_iter = match store_c.iter(&cp) {
                Some(i) => i,
                None => return,
            };
            let child_type: String = store_c.get::<String>(&child_iter, COL_TYPE as i32);
            if child_type != LOADING_SENTINEL {
                return;
            }

            let state_code: String = store_c.get::<String>(&parent_iter, COL_ID as i32);
            if state_code.is_empty() {
                return;
            }

            status_c.set_text(&format!("Loading {}…", state_code));

            let store_rc = store_c.clone();
            let state_rc = Rc::clone(&state_c);
            let status_rc = status_c.clone();
            let cfg_rc = Rc::clone(&cfg_c);
            let path_rc = path.clone(); // store path
            let tv_rc = tv.clone();
            let sc = state_code.clone();
            let filter_model_x = filter_model_r.clone();

            runtime::spawn(
                async move {
                    let client = meso_data::http::wx_client();
                    observations::fetch_metars_for_state(&client, &sc).await
                },
                move |result| {
                    match result {
                        Ok(obs) => {
                            let n = obs.len();
                            status_rc.set_text(&format!("{} stations in {}", n, state_code));

                            let parent_iter = match store_rc.iter(&path_rc) {
                                Some(i) => i,
                                None => return,
                            };
                            // Remove loading sentinel
                            let mut cp = path_rc.clone();
                            cp.append_index(0);
                            if let Some(ci) = store_rc.iter(&cp) {
                                store_rc.remove(&ci);
                            }

                            let favorites = cfg_rc.borrow().obs_favorites.clone();

                            // Sort and insert station rows
                            let mut sorted = obs.clone();
                            sorted.sort_by(|a, b| {
                                let na = if a.station_name.is_empty() {
                                    &a.station_id
                                } else {
                                    &a.station_name
                                };
                                let nb = if b.station_name.is_empty() {
                                    &b.station_id
                                } else {
                                    &b.station_name
                                };
                                na.cmp(nb)
                            });
                            for ob in &sorted {
                                let star = if favorites.contains(&ob.station_id) {
                                    "⭐ "
                                } else {
                                    ""
                                };
                                let display = station_label(ob, star);
                                let row = store_rc.append(Some(&parent_iter));
                                store_rc.set(
                                    &row,
                                    &[
                                        (COL_LABEL, &display.as_str()),
                                        (COL_ID, &ob.station_id.as_str()),
                                        (COL_TYPE, &"station"),
                                    ],
                                );
                            }

                            // Merge into loaded map
                            let mut s = state_rc.borrow_mut();
                            for ob in obs {
                                s.loaded.insert(ob.station_id.clone(), ob);
                            }
                            drop(s);

                            refresh_favorites(&store_rc, &state_rc.borrow(), &cfg_rc.borrow());
                            // Convert store path back to filter model path for expand_row
                            if let Some(fp) = filter_model_x.convert_child_path_to_path(&path_rc) {
                                tv_rc.expand_row(&fp, false);
                            }
                        }
                        Err(e) => {
                            status_rc.set_text(&format!("Load failed: {e}"));
                            tracing::error!("Obs state fetch error: {e}");
                        }
                    }
                },
            );
        });
    }

    // ── cursor-changed: detail + TAF ─────────────────────────────────────────
    {
        let store_c = store.clone();
        let state_c = Rc::clone(&state);
        let detail_box_c = detail_box.clone();
        let cfg_c = Rc::clone(&shared_cfg);
        let store_fav = store.clone();
        let state_fav = Rc::clone(&state);
        let status_c = status_label.clone();
        let filter_model_d = filter_model.clone();

        tree_view.connect_cursor_changed(move |tv| {
            let (filter_path, _) = gtk4::prelude::TreeViewExt::cursor(tv);
            let filter_path = match filter_path {
                Some(p) => p,
                None => return,
            };
            let path = match filter_model_d.convert_path_to_child_path(&filter_path) {
                Some(p) => p,
                None => return,
            };
            let iter = match store_c.iter(&path) {
                Some(i) => i,
                None => return,
            };

            let node_type: String = store_c.get::<String>(&iter, COL_TYPE as i32);
            if node_type != "station" {
                return;
            }

            let station_id: String = store_c.get::<String>(&iter, COL_ID as i32);
            if station_id.is_empty() {
                return;
            }

            let ob = match state_c.borrow().loaded.get(&station_id).cloned() {
                Some(o) => o,
                None => return,
            };

            state_c.borrow_mut().selected_station = Some(station_id.clone());
            let is_fav = cfg_c.borrow().obs_favorites.contains(&station_id);

            // Clear detail and render METAR section
            while let Some(c) = detail_box_c.first_child() {
                detail_box_c.remove(&c);
            }
            render_metar_detail(&detail_box_c, &ob, is_fav, {
                let cfg_cc = Rc::clone(&cfg_c);
                let store_cc = store_fav.clone();
                let state_cc = Rc::clone(&state_fav);
                let sid = station_id.clone();
                move || {
                    toggle_favorite(&cfg_cc, &store_cc, &state_cc.borrow(), &sid);
                }
            });

            // TAF placeholder
            let taf_box = GBox::new(Orientation::Vertical, 4);
            let taf_lbl = Label::new(Some("Fetching TAF…"));
            taf_lbl.add_css_class("dim-label");
            taf_lbl.set_halign(gtk4::Align::Start);
            taf_box.append(&taf_lbl);
            detail_box_c.append(&taf_box);

            let taf_box_c = taf_box.clone();
            let sid = station_id.clone();
            let _sc = status_c.clone();
            runtime::spawn(
                async move {
                    let client = meso_data::http::wx_client();
                    observations::fetch_taf(&client, &sid).await
                },
                move |result| {
                    while let Some(c) = taf_box_c.first_child() {
                        taf_box_c.remove(&c);
                    }
                    match result {
                        Ok(Some(taf)) => render_taf_section(&taf_box_c, &taf),
                        Ok(None) => {
                            let l = Label::new(Some("No TAF available for this station"));
                            l.add_css_class("dim-label");
                            l.set_halign(gtk4::Align::Start);
                            taf_box_c.append(&l);
                        }
                        Err(e) => {
                            tracing::warn!("TAF fetch error: {e}");
                            let l = Label::new(Some(&format!("TAF unavailable: {e}")));
                            l.add_css_class("dim-label");
                            l.set_halign(gtk4::Align::Start);
                            taf_box_c.append(&l);
                        }
                    }
                },
            );
        });
    }

    // ── Preload favorites on startup ──────────────────────────────────────────
    {
        let favorites = shared_cfg.borrow().obs_favorites.clone();
        if !favorites.is_empty() {
            let store_c = store.clone();
            let state_c = Rc::clone(&state);
            let status_c = status_label.clone();
            let favs = favorites.clone();
            let favs2 = favs.clone();
            runtime::spawn(
                async move {
                    let client = meso_data::http::wx_client();
                    let names: std::collections::HashMap<String, String> =
                        std::collections::HashMap::new();
                    observations::fetch_metars(&client, &favs, &names).await
                },
                move |result| {
                    match result {
                        Ok(obs) => {
                            {
                                let mut s = state_c.borrow_mut();
                                for ob in obs {
                                    s.loaded.insert(ob.station_id.clone(), ob);
                                }
                            }
                            // Populate favorites row directly
                            if let Some(fi) = store_c.iter_first() {
                                while let Some(child) = store_c.iter_children(Some(&fi)) {
                                    store_c.remove(&child);
                                }
                                let loaded = state_c.borrow();
                                for sid in &favs2 {
                                    if let Some(ob) = loaded.loaded.get(sid.as_str()) {
                                        let display = station_label(ob, "⭐ ");
                                        let row = store_c.append(Some(&fi));
                                        store_c.set(
                                            &row,
                                            &[
                                                (COL_LABEL, &display.as_str()),
                                                (COL_ID, &ob.station_id.as_str()),
                                                (COL_TYPE, &"station"),
                                            ],
                                        );
                                    }
                                }
                                status_c.set_text(&format!("{} favorite(s) loaded", favs2.len()));
                            }
                        }
                        Err(e) => tracing::warn!("Favorites preload error: {e}"),
                    }
                },
            );
        }
    }

    vbox
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn station_label(ob: &Observation, prefix: &str) -> String {
    if ob.station_name.is_empty() {
        format!("{}{}", prefix, ob.station_id)
    } else {
        format!("{}{} — {}", prefix, ob.station_id, ob.station_name)
    }
}

fn refresh_favorites(store: &TreeStore, obs_state: &ObsState, cfg: &Config) {
    let fav_iter = match store.iter_first() {
        Some(i) if store.get::<String>(&i, COL_TYPE as i32) == "favorites" => i,
        _ => return,
    };
    while let Some(child) = store.iter_children(Some(&fav_iter)) {
        store.remove(&child);
    }
    for sid in &cfg.obs_favorites {
        if let Some(ob) = obs_state.loaded.get(sid.as_str()) {
            let display = station_label(ob, "⭐ ");
            let row = store.append(Some(&fav_iter));
            store.set(
                &row,
                &[
                    (COL_LABEL, &display.as_str()),
                    (COL_ID, &ob.station_id.as_str()),
                    (COL_TYPE, &"station"),
                ],
            );
        }
    }
}

fn toggle_favorite(cfg: &Rc<RefCell<Config>>, store: &TreeStore, obs_state: &ObsState, sid: &str) {
    {
        let mut c = cfg.borrow_mut();
        if c.obs_favorites.contains(&sid.to_string()) {
            c.obs_favorites.retain(|s| s != sid);
        } else {
            c.obs_favorites.push(sid.to_string());
        }
        let _ = c.save();
    }
    refresh_favorites(store, obs_state, &cfg.borrow());
}

// ── METAR detail ──────────────────────────────────────────────────────────────

fn render_metar_detail(
    detail_box: &GBox,
    ob: &Observation,
    is_fav: bool,
    on_fav_toggle: impl Fn() + 'static,
) {
    let header_box = GBox::new(Orientation::Horizontal, 8);
    let id_lbl = Label::new(Some(&ob.station_id));
    id_lbl.add_css_class("obs-detail-id");
    header_box.append(&id_lbl);
    if !ob.station_name.is_empty() {
        header_box.append(&Label::new(Some(&format!("  {}", ob.station_name))));
    }
    let fav_btn = Button::with_label(if is_fav {
        "⭐ Unfavorite"
    } else {
        "☆ Favorite"
    });
    fav_btn.add_css_class("flat");
    {
        let fav_btn_c = fav_btn.clone();
        let is_fav_state = Rc::new(Cell::new(is_fav));
        fav_btn.connect_clicked(move |_| {
            on_fav_toggle();
            let new_fav = !is_fav_state.get();
            is_fav_state.set(new_fav);
            fav_btn_c.set_label(if new_fav {
                "⭐ Unfavorite"
            } else {
                "☆ Favorite"
            });
        });
    }
    header_box.append(&fav_btn);
    let time_lbl = Label::new(Some(&time_ago_str(&ob.obs_time)));
    time_lbl.set_hexpand(true);
    time_lbl.set_halign(gtk4::Align::End);
    time_lbl.add_css_class("dim-label");
    header_box.append(&time_lbl);
    detail_box.append(&header_box);

    if let Some(cat) = &ob.flight_category {
        let cl = Label::new(Some(cat));
        cl.set_halign(gtk4::Align::Start);
        let css = match cat.as_str() {
            "LIFR" => "obs-cat-lifr",
            "IFR" => "obs-cat-ifr",
            "MVFR" => "obs-cat-mvfr",
            _ => "obs-cat-vfr",
        };
        cl.add_css_class(css);
        detail_box.append(&cl);
    }

    detail_box.append(&Separator::new(Orientation::Horizontal));

    let grid = gtk4::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(6);
    grid.set_margin_top(8);
    let mut row = 0i32;

    let temp_str = match (ob.temp_f, ob.dew_f) {
        (Some(t), Some(d)) => format!("{:.1}°F  /  Dewpoint: {:.1}°F", t, d),
        (Some(t), None) => format!("{:.1}°F", t),
        _ => "N/A".to_string(),
    };
    add_detail_row(&grid, row, "Temperature", &temp_str);
    row += 1;
    add_detail_row(&grid, row, "Wind", &format_wind(ob));
    row += 1;
    let vis_str = match ob.visibility_mi {
        Some(v) if v >= 10.0 => "10+ SM".to_string(),
        Some(v) => format!("{:.2} SM", v),
        None => "N/A".to_string(),
    };
    add_detail_row(&grid, row, "Visibility", &vis_str);
    row += 1;
    add_detail_row(
        &grid,
        row,
        "Sky",
        if ob.sky_cover.is_empty() {
            "N/A"
        } else {
            &ob.sky_cover
        },
    );
    row += 1;
    let altim_str = ob
        .altimeter_inhg
        .map(|a| format!("{:.2} inHg", a))
        .unwrap_or_else(|| "N/A".to_string());
    add_detail_row(&grid, row, "Altimeter", &altim_str);
    let _ = row;
    detail_box.append(&grid);

    detail_box.append(&Separator::new(Orientation::Horizontal));

    let raw_hdr = Label::new(Some("Raw METAR"));
    raw_hdr.set_halign(gtk4::Align::Start);
    raw_hdr.add_css_class("heading");
    detail_box.append(&raw_hdr);

    if ob.raw_metar.is_empty() {
        let l = Label::new(Some("(not available)"));
        l.add_css_class("dim-label");
        detail_box.append(&l);
    } else {
        let l = Label::new(Some(&ob.raw_metar));
        l.set_selectable(true);
        l.set_wrap(true);
        l.set_wrap_mode(gtk4::pango::WrapMode::Char);
        l.set_halign(gtk4::Align::Start);
        l.add_css_class("obs-raw-metar");
        detail_box.append(&l);
    }

    detail_box.append(&Separator::new(Orientation::Horizontal));
}

// ── TAF detail ────────────────────────────────────────────────────────────────

fn render_taf_section(container: &GBox, taf: &Taf) {
    let hdr = Label::new(Some("TAF — Terminal Aerodrome Forecast"));
    hdr.set_halign(gtk4::Align::Start);
    hdr.add_css_class("heading");
    container.append(&hdr);

    let valid_lbl = Label::new(Some(&format!(
        "Valid {} – {}",
        unix_short(taf.valid_from),
        unix_short(taf.valid_to)
    )));
    valid_lbl.set_halign(gtk4::Align::Start);
    valid_lbl.add_css_class("dim-label");
    container.append(&valid_lbl);

    let raw_hdr = Label::new(Some("Raw TAF"));
    raw_hdr.set_halign(gtk4::Align::Start);
    raw_hdr.add_css_class("heading");
    container.append(&raw_hdr);

    if !taf.raw_taf.is_empty() {
        // Insert line breaks before each change group if the API didn't include them.
        let formatted = add_taf_linebreaks(&taf.raw_taf);
        let l = Label::new(Some(&formatted));
        l.set_selectable(true);
        l.set_wrap(true);
        l.set_wrap_mode(gtk4::pango::WrapMode::Char);
        l.set_halign(gtk4::Align::Start);
        l.add_css_class("obs-raw-metar");
        container.append(&l);
    }

    container.append(&Separator::new(Orientation::Horizontal));

    let translated_hdr = Label::new(Some("Translated TAF"));
    translated_hdr.set_halign(gtk4::Align::Start);
    translated_hdr.add_css_class("heading");
    container.append(&translated_hdr);

    for period in &taf.periods {
        let pb = GBox::new(Orientation::Vertical, 2);
        pb.set_margin_top(6);

        let change = period.change_type.as_deref().unwrap_or("FROM");
        let thdr = Label::new(Some(&format!(
            "{} {} – {}",
            change,
            unix_short(period.time_from),
            unix_short(period.time_to)
        )));
        thdr.set_halign(gtk4::Align::Start);
        thdr.add_css_class("heading");
        pb.append(&thdr);

        let g = gtk4::Grid::new();
        g.set_column_spacing(10);
        g.set_row_spacing(4);
        let mut r = 0i32;

        let wind_str = match (period.wind_dir, period.wind_speed) {
            (None, _) | (_, None) => String::new(),
            (Some(0), Some(0)) => "Calm".to_string(),
            (Some(dir), Some(spd)) => {
                let cmp = deg_to_compass(dir);
                let gust = period
                    .wind_gust
                    .map(|g| format!(" G{g}kt"))
                    .unwrap_or_default();
                format!("{cmp} ({dir}°) at {spd}kt{gust}")
            }
        };
        if !wind_str.is_empty() {
            add_detail_row(&g, r, "Wind", &wind_str);
            r += 1;
        }

        if let Some(vis) = &period.visibility {
            let vd = if vis == "6+" {
                "6+ SM".to_string()
            } else {
                format!("{vis} SM")
            };
            add_detail_row(&g, r, "Visibility", &vd);
            r += 1;
        }
        if let Some(wx) = &period.wx_string {
            add_detail_row(&g, r, "Weather", wx);
            r += 1;
        }
        if !period.sky_cover.is_empty() && period.sky_cover != "CLR" {
            add_detail_row(&g, r, "Sky", &period.sky_cover);
        }
        let _ = r;
        pb.append(&g);
        container.append(&pb);
    }
}

fn add_taf_linebreaks(raw: &str) -> String {
    // Aviation TAF change groups that start a new line in standard formatting.
    // Insert a newline before each one (whether or not the API already included one).
    let normalized = raw.replace('\r', "").replace('\n', " ");
    let mut out = normalized.clone();
    for kw in &["FM", "TEMPO", "BECMG", "PROB30", "PROB40"] {
        // Replace " FM" with "\nFM" etc., but be careful not to duplicate existing newlines.
        let pat = format!(" {kw}");
        let rep = format!("\n{kw}");
        out = out.replace(&pat, &rep);
    }
    out
}

fn unix_short(ts: i64) -> String {
    if ts == 0 {
        return "--".to_string();
    }
    let s = ts as u64;
    let mins = s / 60;
    let minute = mins % 60;
    let hours = mins / 60;
    let hour = hours % 24;
    let day = (hours / 24) % 31 + 1;
    format!("{:02}/{:02}:{:02}Z", day, hour, minute)
}

fn add_detail_row(grid: &gtk4::Grid, row: i32, label: &str, value: &str) {
    let l = Label::new(Some(label));
    l.set_halign(gtk4::Align::End);
    l.add_css_class("dim-label");
    let v = Label::new(Some(value));
    v.set_halign(gtk4::Align::Start);
    v.set_selectable(true);
    grid.attach(&l, 0, row, 1, 1);
    grid.attach(&v, 1, row, 1, 1);
}

fn format_wind(ob: &Observation) -> String {
    match (ob.wind_dir, ob.wind_speed_kt) {
        (None, _) | (_, None) => "Calm".to_string(),
        (Some(0), Some(0)) => "Calm".to_string(),
        (Some(dir), Some(spd)) => {
            let cmp = deg_to_compass(dir);
            let gust = ob
                .wind_gust_kt
                .map(|g| format!(" (gusts {g}kt)"))
                .unwrap_or_default();
            format!("{cmp} ({dir}°) at {spd}kt{gust}")
        }
    }
}
