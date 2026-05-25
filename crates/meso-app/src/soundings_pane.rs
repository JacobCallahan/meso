/*
 * Upper-air soundings viewer pane.
 *
 * Layout (horizontal Paned):
 *   Left  — image viewer (Skew-T DrawingArea) + toolbar + status
 *   Right — station list (TreeView grouped by state) + search + favorites
 *
 * Data source:
 *   SPC experimental sounding page (pre-rendered Skew-T GIF + derived indices text)
 *   https://www.spc.noaa.gov/exper/soundings/LATEST/{SITE}.gif
 */

use glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, CellRendererText, DrawingArea, Label, Orientation, Paned, PolicyType,
    ScrolledWindow, SearchEntry, ToggleButton, TreeModelFilter, TreeStore, TreeView, TreeViewColumn,
};

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use meso_data::soundings::{self, SoundingSite};

use crate::config::Config;
use crate::runtime;
use crate::ui::enable_status_copy;

// ── State ─────────────────────────────────────────────────────────────────────

struct SoundingState {
    current_site_id: String,
    current_site_name: String,
    current_pixbuf: Option<gdk_pixbuf::Pixbuf>,
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
}

impl Default for SoundingState {
    fn default() -> Self {
        SoundingState {
            current_site_id: String::new(),
            current_site_name: String::new(),
            current_pixbuf: None,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        }
    }
}

// ── Public builder ────────────────────────────────────────────────────────────

pub fn build_soundings_pane(shared_config: Rc<RefCell<Config>>) -> GBox {
    let state = Rc::new(RefCell::new(SoundingState::default()));

    let outer = GBox::new(Orientation::Vertical, 0);

    let paned = Paned::new(Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_hexpand(true);

    let saved_pos = shared_config.borrow().soundings_pane_position;
    if saved_pos > 0 {
        paned.set_position(saved_pos);
    } else {
        let p_clone = paned.clone();
        paned.connect_realize(move |_| {
            let w = p_clone.width();
            if w > 10 {
                p_clone.set_position((w as f64 * 0.74) as i32);
            }
        });
    }
    {
        let cfg = Rc::clone(&shared_config);
        paned.connect_position_notify(move |p| {
            cfg.borrow_mut().soundings_pane_position = p.position();
        });
    }

    // ── Left: toolbar + image viewer + status ─────────────────────────────────
    let left = GBox::new(Orientation::Vertical, 0);

    // Toolbar
    let toolbar = GBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);

    let refresh_btn = Button::with_label("⟳ Refresh");
    refresh_btn.set_tooltip_text(Some("Reload this sounding"));
    toolbar.append(&refresh_btn);
    left.append(&toolbar);

    // Drawing area (Skew-T image)
    let drawing_area = DrawingArea::new();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);

    {
        let state_d = Rc::clone(&state);
        drawing_area.set_draw_func(move |_da, cr, w, h| {
            let st = state_d.borrow();
            let widget_w = w as f64;
            let widget_h = h as f64;

            // Sounding imagery has dark annotations; keep a white backdrop for readability.
            cr.set_source_rgb(1.0, 1.0, 1.0);
            let _ = cr.paint();

            if let Some(pb) = &st.current_pixbuf {
                let img_w = pb.width() as f64;
                let img_h = pb.height() as f64;
                let fit = (widget_w / img_w).min(widget_h / img_h);
                let scale = fit * st.zoom;
                let x = (widget_w - img_w * scale) / 2.0 - st.pan_x * scale;
                let y = (widget_h - img_h * scale) / 2.0 - st.pan_y * scale;
                cr.translate(x, y);
                cr.scale(scale, scale);
                cr.set_source_pixbuf(pb, 0.0, 0.0);
                let _ = cr.paint();
            } else if st.current_site_id.is_empty() {
                let msg = "Select a station →";
                cr.set_source_rgb(0.7, 0.7, 0.7);
                cr.select_font_face(
                    "Sans",
                    gtk4::cairo::FontSlant::Normal,
                    gtk4::cairo::FontWeight::Normal,
                );
                cr.set_font_size(14.0);
                if let Ok(ext) = cr.text_extents(msg) {
                    cr.move_to((widget_w - ext.width()) / 2.0, widget_h / 2.0);
                    let _ = cr.show_text(msg);
                }
            }
        });
    }

    // Zoom via scroll
    {
        let state_z = Rc::clone(&state);
        let da_z = drawing_area.clone();
        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |_, _dx, dy| {
            let factor = if dy < 0.0 { 1.15 } else { 1.0 / 1.15 };
            zoom_around(&mut state_z.borrow_mut(), factor, &da_z);
            da_z.queue_draw();
            glib::Propagation::Stop
        });
        drawing_area.add_controller(scroll);
    }

    // Drag to pan
    {
        let state_p = Rc::clone(&state);
        let da_p = drawing_area.clone();
        let pan_origin: Rc<RefCell<(f64, f64)>> = Rc::new(RefCell::new((0.0, 0.0)));

        let drag = gtk4::GestureDrag::new();
        {
            let po = Rc::clone(&pan_origin);
            let st_c = Rc::clone(&state_p);
            drag.connect_drag_begin(move |_, _x, _y| {
                let st = st_c.borrow();
                *po.borrow_mut() = (st.pan_x, st.pan_y);
            });
        }
        {
            let po = Rc::clone(&pan_origin);
            let st_c = Rc::clone(&state_p);
            let da_c = da_p.clone();
            drag.connect_drag_update(move |_, ox, oy| {
                let (origin_x, origin_y) = *po.borrow();
                let pb_size = st_c
                    .borrow()
                    .current_pixbuf
                    .as_ref()
                    .map(|pb| (pb.width() as f64, pb.height() as f64))
                    .unwrap_or((1.0, 1.0));
                let fit = (da_c.width() as f64 / pb_size.0).min(da_c.height() as f64 / pb_size.1);
                let scale = fit * st_c.borrow().zoom;
                let mut st = st_c.borrow_mut();
                st.pan_x = origin_x - ox / scale;
                st.pan_y = origin_y - oy / scale;
                drop(st);
                da_c.queue_draw();
            });
        }
        drawing_area.add_controller(drag);
    }

    left.append(&drawing_area);

    // Status label
    let status = Label::new(Some("Select a station from the list →"));
    status.set_halign(gtk4::Align::Start);
    status.set_margin_start(4);
    status.set_margin_bottom(2);
    enable_status_copy(&status);
    left.append(&status);

    paned.set_start_child(Some(&left));

    // ── Right: search + favorites + station tree ──────────────────────────────
    let right_box = GBox::new(Orientation::Vertical, 0);

    let tree_toolbar = GBox::new(Orientation::Horizontal, 4);
    tree_toolbar.set_margin_start(4);
    tree_toolbar.set_margin_end(4);
    tree_toolbar.set_margin_top(4);
    tree_toolbar.set_margin_bottom(2);

    let search_entry = SearchEntry::new();
    search_entry.set_hexpand(true);
    search_entry.set_placeholder_text(Some("Filter stations…"));

    let fav_btn = ToggleButton::new();
    fav_btn.set_label("☆");
    fav_btn.set_tooltip_text(Some("Toggle favorite station"));
    fav_btn.set_sensitive(false);
    let suppress_fav_toggle = Rc::new(Cell::new(false));

    tree_toolbar.append(&search_entry);
    tree_toolbar.append(&fav_btn);
    right_box.append(&tree_toolbar);

    let tree_scroll = ScrolledWindow::new();
    tree_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    tree_scroll.set_vexpand(true);
    tree_scroll.set_hexpand(false);
    tree_scroll.set_min_content_width(260);

    // TreeStore: col 0 = display label, col 1 = site_id, col 2 = visible
    let store = TreeStore::new(&[glib::Type::STRING, glib::Type::STRING, glib::Type::BOOL]);

    // ── Favorites category ────────────────────────────────────────────────────
    let fav_cat_iter = store.append(None);
    store.set(&fav_cat_iter, &[(0, &"⭐ Favorites"), (1, &""), (2, &true)]);
    {
        let favs = shared_config.borrow().sounding_favorites.clone();
        for fav_id in &favs {
            if let Some(site) = soundings::SITES.iter().find(|s| s.id == fav_id.as_str()) {
                let iter = store.append(Some(&fav_cat_iter));
                let label = format!("{} — {}", site.id, site.name);
                store.set(&iter, &[(0, &label.as_str()), (1, &site.id), (2, &true)]);
            }
        }
    }
    let fav_has_children = store.iter_has_child(&fav_cat_iter);
    store.set(&fav_cat_iter, &[(2, &fav_has_children)]);

    // ── Stations grouped by state ─────────────────────────────────────────────
    // Build a sorted state → sites map
    let mut by_state: Vec<(String, Vec<&'static SoundingSite>)> = {
        use std::collections::BTreeMap;
        let mut map: BTreeMap<String, Vec<&SoundingSite>> = BTreeMap::new();
        for site in soundings::SITES {
            // Extract state/country prefix (text before first comma)
            let region = site.name.split(',').next().unwrap_or("Other").trim();
            map.entry(region.to_string()).or_default().push(site);
        }
        map.into_iter().collect()
    };
    // Sort sites within each state alphabetically by name
    for (_, sites) in &mut by_state {
        sites.sort_by_key(|s| s.name);
    }

    for (region, sites) in &by_state {
        let cat_iter = store.append(None);
        store.set(&cat_iter, &[(0, &region.as_str()), (1, &""), (2, &true)]);
        for site in sites {
            let iter = store.append(Some(&cat_iter));
            let label = format!("{} — {}", site.id, &site.name[site.name.find(',').map(|i| i + 2).unwrap_or(0)..]);
            store.set(&iter, &[(0, &label.as_str()), (1, &site.id), (2, &true)]);
        }
    }

    let filter = TreeModelFilter::new(&store, None);
    filter.set_visible_column(2);

    let tree_view = TreeView::with_model(&filter);
    tree_view.set_headers_visible(false);
    tree_view.set_enable_tree_lines(true);
    tree_view.set_activate_on_single_click(true);

    let renderer = CellRendererText::new();
    let col = TreeViewColumn::new();
    col.pack_start(&renderer, true);
    col.add_attribute(&renderer, "text", 0);
    tree_view.append_column(&col);

    // Collapse categories by default (many states — expand on search or click)
    tree_view.collapse_all();

    // ── Search filtering ──────────────────────────────────────────────────────
    {
        let store_s = store.clone();
        let filter_s = filter.clone();
        let tv_s = tree_view.clone();
        let fav_path = store.path(&fav_cat_iter);
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_lowercase();
            if let Some(cat_iter) = store_s.iter_first() {
                loop {
                    let cat_id: String = store_s.get::<String>(&cat_iter, 1);
                    if cat_id.is_empty() {
                        let mut any = false;
                        if let Some(child) = store_s.iter_children(Some(&cat_iter)) {
                            loop {
                                let label: String = store_s.get::<String>(&child, 0);
                                let visible =
                                    query.is_empty() || label.to_lowercase().contains(&query);
                                store_s.set(&child, &[(2, &visible)]);
                                if visible { any = true; }
                                if !store_s.iter_next(&child) { break; }
                            }
                        }
                        let this_path = store_s.path(&cat_iter);
                        let is_fav = fav_path == this_path;
                        let cat_visible = if is_fav {
                            store_s.iter_has_child(&cat_iter)
                        } else {
                            any || query.is_empty()
                        };
                        store_s.set(&cat_iter, &[(2, &cat_visible)]);
                    }
                    if !store_s.iter_next(&cat_iter) { break; }
                }
            }
            filter_s.refilter();
            if query.is_empty() {
                tv_s.collapse_all();
            } else {
                tv_s.expand_all();
            }
        });
    }

    tree_scroll.set_child(Some(&tree_view));
    right_box.append(&tree_scroll);

    paned.set_end_child(Some(&right_box));
    outer.append(&paned);

    // ── Row activated → load sounding ────────────────────────────────────────
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let fav_btn_c = fav_btn.clone();
        let suppress_fav_toggle_c = Rc::clone(&suppress_fav_toggle);
        let cfg_c = Rc::clone(&shared_config);
        let refresh_btn_c = refresh_btn.clone();

        tree_view.connect_row_activated(move |tv, path, _col| {
            let model = tv.model().unwrap();
            let iter = model.iter(path).unwrap();
            let site_id: String = model.get::<String>(&iter, 1);
            let label: String = model.get::<String>(&iter, 0);
            if site_id.is_empty() { return; }

            {
                let mut st = state_c.borrow_mut();
                st.current_site_id = site_id.clone();
                st.current_site_name = label.clone();
                st.current_pixbuf = None;
                st.zoom = 1.0;
                st.pan_x = 0.0;
                st.pan_y = 0.0;
            }
            cfg_c.borrow_mut().sounding_last_site = site_id.clone();

            let is_fav = cfg_c.borrow().sounding_favorites.contains(&site_id);
            suppress_fav_toggle_c.set(true);
            fav_btn_c.set_active(is_fav);
            fav_btn_c.set_label(if is_fav { "★" } else { "☆" });
            fav_btn_c.set_sensitive(true);
            suppress_fav_toggle_c.set(false);

            load_sounding(
                site_id,
                Rc::clone(&state_c),
                da_c.clone(),
                st_c.clone(),
                vec![refresh_btn_c.clone()],
            );
        });
    }

    // ── Refresh button ────────────────────────────────────────────────────────
    {
        let state_r = Rc::clone(&state);
        let da_r = drawing_area.clone();
        let st_r = status.clone();

        refresh_btn.connect_clicked(move |btn| {
            let site_id = state_r.borrow().current_site_id.clone();
            if site_id.is_empty() { return; }

            // Bust cache by clearing soundings namespace entries for this site
            let cache = meso_data::cache::Cache::new("soundings");
            cache.invalidate(&soundings::image_url(&site_id));
            cache.invalidate(&soundings::text_url(&site_id));

            {
                let mut st = state_r.borrow_mut();
                st.current_pixbuf = None;
            }
            da_r.queue_draw();

            load_sounding(
                site_id,
                Rc::clone(&state_r),
                da_r.clone(),
                st_r.clone(),
                vec![btn.clone()],
            );
        });
    }

    // ── Favorite toggle ───────────────────────────────────────────────────────
    {
        let state_f = Rc::clone(&state);
        let store_f = store.clone();
        let filter_f = filter.clone();
        let fav_iter_f = fav_cat_iter.clone();
        let cfg_f = Rc::clone(&shared_config);
        let tv_f = tree_view.clone();
        let suppress_fav_toggle_f = Rc::clone(&suppress_fav_toggle);

        fav_btn.connect_toggled(move |btn| {
            if suppress_fav_toggle_f.get() { return; }
            let site_id = state_f.borrow().current_site_id.clone();
            if site_id.is_empty() { return; }
            let site_name = state_f.borrow().current_site_name.clone();
            let is_now_fav = btn.is_active();
            btn.set_label(if is_now_fav { "★" } else { "☆" });

            let mut cfg = cfg_f.borrow_mut();
            if is_now_fav {
                if !cfg.sounding_favorites.contains(&site_id) {
                    cfg.sounding_favorites.push(site_id.clone());
                    let iter = store_f.append(Some(&fav_iter_f));
                    store_f.set(
                        &iter,
                        &[(0, &site_name.as_str()), (1, &site_id.as_str()), (2, &true)],
                    );
                    store_f.set(&fav_iter_f, &[(2, &true)]);
                    filter_f.refilter();
                    let path = store_f.path(&fav_iter_f);
                    tv_f.expand_row(&path, false);
                }
            } else {
                cfg.sounding_favorites.retain(|id| id != &site_id);
                if let Some(child) = store_f.iter_children(Some(&fav_iter_f)) {
                    loop {
                        let id: String = store_f.get::<String>(&child, 1);
                        if id == site_id {
                            store_f.remove(&child);
                            break;
                        }
                        if !store_f.iter_next(&child) { break; }
                    }
                }
                let still_has = store_f.iter_has_child(&fav_iter_f);
                store_f.set(&fav_iter_f, &[(2, &still_has)]);
                filter_f.refilter();
            }
        });
    }

    // ── Auto-select nearest station on first show ─────────────────────────────
    {
        let cfg_auto = Rc::clone(&shared_config);
        let state_auto = Rc::clone(&state);
        let da_auto = drawing_area.clone();
        let st_auto = status.clone();
        let refresh_btn_auto = refresh_btn.clone();
        let tv_auto = tree_view.clone();
        let store_auto = store.clone();
        let filter_auto = filter.clone();
        let fav_btn_auto = fav_btn.clone();
        let suppress_fav_toggle_auto = Rc::clone(&suppress_fav_toggle);

        outer.connect_map(move |_| {
            // Only auto-select once (when state is empty)
            if !state_auto.borrow().current_site_id.is_empty() { return; }

            let cfg = cfg_auto.borrow();
            let last = cfg.sounding_last_site.clone();
            let site_id = if !last.is_empty() && soundings::SITES.iter().any(|s| s.id == last.as_str()) {
                last
            } else {
                soundings::nearest_site(cfg.location_lat, cfg.location_lon).id.to_string()
            };
            drop(cfg);

            let is_fav = cfg_auto.borrow().sounding_favorites.contains(&site_id);
            suppress_fav_toggle_auto.set(true);
            fav_btn_auto.set_active(is_fav);
            fav_btn_auto.set_label(if is_fav { "★" } else { "☆" });
            fav_btn_auto.set_sensitive(true);
            suppress_fav_toggle_auto.set(false);

            cfg_auto.borrow_mut().sounding_last_site = site_id.clone();
            {
                let site_name = soundings::SITES
                    .iter()
                    .find(|s| s.id == site_id.as_str())
                    .map(|s| s.name)
                    .unwrap_or(&site_id);
                let mut st = state_auto.borrow_mut();
                st.current_site_id = site_id.clone();
                st.current_site_name = site_name.to_string();
            }

            // Scroll the tree to and select the matching row
            select_tree_row(&tv_auto, &store_auto, &filter_auto, &site_id);

            load_sounding(
                site_id,
                Rc::clone(&state_auto),
                da_auto.clone(),
                st_auto.clone(),
                vec![refresh_btn_auto.clone()],
            );
        });
    }

    outer
}

// ── Helper: select a tree row by site_id ──────────────────────────────────────

fn select_tree_row(
    tv: &TreeView,
    store: &TreeStore,
    filter: &TreeModelFilter,
    site_id: &str,
) {
    if let Some(cat_iter) = store.iter_first() {
        loop {
            if let Some(child) = store.iter_children(Some(&cat_iter)) {
                loop {
                    let id: String = store.get::<String>(&child, 1);
                    if id == site_id {
                        // Expand category and scroll to row
                        let store_path = store.path(&cat_iter);
                        // Map through filter
                        if let Some(fpath) = filter.convert_child_path_to_path(&store_path) {
                            tv.expand_row(&fpath, false);
                        }
                        let child_path = store.path(&child);
                        if let Some(fchild_path) = filter.convert_child_path_to_path(&child_path) {
                            gtk4::prelude::TreeViewExt::set_cursor(tv, &fchild_path, None::<&TreeViewColumn>, false);
                            tv.scroll_to_cell(Some(&fchild_path), None::<&TreeViewColumn>, false, 0.0, 0.0);
                        }
                        return;
                    }
                    if !store.iter_next(&child) { break; }
                }
            }
            if !store.iter_next(&cat_iter) { break; }
        }
    }
}

// ── Zoom helper ────────────────────────────────────────────────────────────────

fn zoom_around(st: &mut SoundingState, factor: f64, da: &DrawingArea) {
    if let Some(pb) = &st.current_pixbuf {
        let img_w = pb.width() as f64;
        let img_h = pb.height() as f64;
        let widget_w = da.width() as f64;
        let widget_h = da.height() as f64;
        let fit = (widget_w / img_w).min(widget_h / img_h);
        let total = fit * st.zoom;
        let cx = widget_w / 2.0;
        let cy = widget_h / 2.0;
        let img_x = (cx - widget_w / 2.0) / total + img_w / 2.0 + st.pan_x;
        let img_y = (cy - widget_h / 2.0) / total + img_h / 2.0 + st.pan_y;
        st.zoom = (st.zoom * factor).clamp(0.1, 20.0);
        let new_total = fit * st.zoom;
        st.pan_x = img_x - img_w / 2.0 - (cx - widget_w / 2.0) / new_total;
        st.pan_y = img_y - img_h / 2.0 - (cy - widget_h / 2.0) / new_total;
    } else {
        st.zoom = (st.zoom * factor).clamp(0.1, 20.0);
    }
}

// ── Async fetch ───────────────────────────────────────────────────────────────

fn load_sounding(
    site_id: String,
    state: Rc<RefCell<SoundingState>>,
    da: DrawingArea,
    status: Label,
    btns: Vec<Button>,
) {
    for b in &btns {
        b.set_sensitive(false);
    }
    status.set_text(&format!("Loading sounding for {}…", site_id));

    let sid = site_id.clone();
    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            let image = soundings::fetch_image(&client, &sid).await?;
            Ok::<_, anyhow::Error>(image)
        },
        move |result| {
            for b in &btns {
                b.set_sensitive(true);
            }
            match result {
                Ok(image_bytes) => {
                    if let Some(pb) = bytes_to_pixbuf(&image_bytes) {
                        let site = state.borrow().current_site_id.clone();
                        let mut st = state.borrow_mut();
                        st.current_pixbuf = Some(pb);
                        drop(st);
                        da.queue_draw();
                        status.set_text(&format!("Sounding: {site}"));
                    } else {
                        status.set_text("Failed to decode sounding image");
                    }
                }
                Err(e) => status.set_text(&format!("Error: {e}")),
            }
        },
    );
}

fn bytes_to_pixbuf(bytes: &[u8]) -> Option<gdk_pixbuf::Pixbuf> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    loader.pixbuf()
}
