/*
 * SPC SREF Models viewer pane.
 *
 * Layout (horizontal Paned):
 *   Left  — image viewer (DrawingArea) + animation controls + status
 *   Right — collapsible TreeView of SREF products (category → product)
 *
 * Interaction:
 *   • Click a product row → load F+000 immediately; "Animate" fetches all 30 frames
 *   • ▶ Animate / ⏸ Pause — same pause/resume semantics as radar
 *   • Timeline Scale — scrub through animation frames
 *   • ⟳ Refresh — clears local cache for current product and re-fetches
 */

use glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, CellRendererText, DrawingArea, DropDown, Label, Orientation, Paned,
    PolicyType, Scale, ScrolledWindow, SearchEntry, StringList, ToggleButton, TreeModelFilter,
    TreeStore, TreeView, TreeViewColumn,
};

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use meso_data::models::{self, ncep};

use crate::config::Config;
use crate::runtime;
use crate::ui::enable_status_copy;

// ── Model type selector ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum ActiveModel {
    Sref,
    Ncep(ncep::NcepModel),
}

impl ActiveModel {
    #[allow(dead_code)]
    fn label(&self) -> &str {
        match self {
            ActiveModel::Sref => "SREF",
            ActiveModel::Ncep(m) => m.label(),
        }
    }
}

// ── State ─────────────────────────────────────────────────────────────────────

struct ModelState {
    current_product_id: String,
    current_product_label: String,
    current_pixbuf: Option<gdk_pixbuf::Pixbuf>,
    // Model run initialization time (UTC) — used to display local valid times
    run_init_time: Option<chrono::DateTime<chrono::Utc>>,
    // Image-space zoom/pan
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
    // Animation
    anim_frames: Vec<gdk_pixbuf::Pixbuf>,
    anim_timestamps: Vec<String>, // local valid-time strings, e.g. "05/23 06:00"
    anim_index: usize,
    // Active model type
    active_model: ActiveModel,
    // NCEP sector (e.g. "CONUS")
    ncep_sector: String,
    // NCEP run time string (YYYYMMDDHH)
    ncep_run: String,
}

impl Default for ModelState {
    fn default() -> Self {
        // Default to first product in catalog
        let cats = models::sref_categories();
        let first = &cats[0].products[0];
        ModelState {
            current_product_id: first.id.to_string(),
            current_product_label: first.label.to_string(),
            current_pixbuf: None,
            run_init_time: None,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            anim_frames: Vec::new(),
            anim_timestamps: Vec::new(),
            anim_index: 0,
            active_model: ActiveModel::Sref,
            ncep_sector: "CONUS".to_string(),
            ncep_run: String::new(),
        }
    }
}

// ── Public builder ────────────────────────────────────────────────────────────

pub fn build_models_pane(shared_config: Rc<RefCell<Config>>) -> GBox {
    // Restore persisted model type and sector
    let saved_model_type = shared_config.borrow().ncep_model_type.clone();
    let saved_sector = shared_config.borrow().ncep_sector.clone();

    let initial_active_model = match saved_model_type.as_str() {
        "gfs"  => ActiveModel::Ncep(ncep::NcepModel::Gfs),
        "nam"  => ActiveModel::Ncep(ncep::NcepModel::Nam),
        "rap"  => ActiveModel::Ncep(ncep::NcepModel::Rap),
        "hrrr" => ActiveModel::Ncep(ncep::NcepModel::Hrrr),
        _      => ActiveModel::Sref,
    };

    let initial_state = {
        let cats = models::sref_categories();
        let first = &cats[0].products[0];
        ModelState {
            current_product_id: first.id.to_string(),
            current_product_label: first.label.to_string(),
            current_pixbuf: None,
            run_init_time: None,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            anim_frames: Vec::new(),
            anim_timestamps: Vec::new(),
            anim_index: 0,
            active_model: initial_active_model.clone(),
            ncep_sector: saved_sector.clone(),
            ncep_run: String::new(),
        }
    };
    let state = Rc::new(RefCell::new(initial_state));
    let anim_running = Rc::new(Cell::new(false));
    let anim_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let slider_updating: Rc<Cell<bool>> = Rc::new(Cell::new(false));

    let outer = GBox::new(Orientation::Vertical, 0);

    let paned = Paned::new(Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_hexpand(true);

    // Restore or auto-fit position (right tree = ~25-28% of width)
    let saved_pos = shared_config.borrow().models_pane_position;
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

    // Save position changes back to config
    {
        let cfg = Rc::clone(&shared_config);
        paned.connect_position_notify(move |p| {
            cfg.borrow_mut().models_pane_position = p.position();
        });
    }

    // ── Left: image viewer + controls ─────────────────────────────────────────
    let left = GBox::new(Orientation::Vertical, 0);

    // Toolbar
    let toolbar = GBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);

    let refresh_btn = Button::with_label("⟳ Refresh");
    let anim_btn = Button::with_label("▶ Animate");
    toolbar.append(&refresh_btn);
    toolbar.append(&anim_btn);
    left.append(&toolbar);

    // Drawing area
    let drawing_area = DrawingArea::new();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);

    // Draw callback
    {
        let state_d = Rc::clone(&state);
        drawing_area.set_draw_func(move |_da, cr, w, h| {
            let st = state_d.borrow();
            let widget_w = w as f64;
            let widget_h = h as f64;

            // Black background
            cr.set_source_rgb(0.0, 0.0, 0.0);
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
            } else {
                let lbl = &st.current_product_label;
                cr.set_source_rgb(0.7, 0.7, 0.7);
                cr.select_font_face(
                    "Sans",
                    gtk4::cairo::FontSlant::Normal,
                    gtk4::cairo::FontWeight::Normal,
                );
                cr.set_font_size(14.0);
                if let Ok(ext) = cr.text_extents(lbl) {
                    cr.move_to((widget_w - ext.width()) / 2.0, widget_h / 2.0);
                    let _ = cr.show_text(lbl);
                }
            }

            // Draw current frame label (top-left)
            if !st.anim_timestamps.is_empty() {
                if let Some(ts) = st.anim_timestamps.get(st.anim_index) {
                    cr.identity_matrix();
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.6);
                    cr.rectangle(4.0, 4.0, 120.0, 22.0);
                    let _ = cr.fill();
                    cr.set_source_rgb(1.0, 1.0, 1.0);
                    cr.select_font_face(
                        "Monospace",
                        gtk4::cairo::FontSlant::Normal,
                        gtk4::cairo::FontWeight::Bold,
                    );
                    cr.set_font_size(12.0);
                    cr.move_to(8.0, 20.0);
                    let _ = cr.show_text(ts);
                }
            }
        });
    }

    // Scroll to zoom
    {
        let state_z = Rc::clone(&state);
        let da_z = drawing_area.clone();
        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |_, _dx, dy| {
            let factor = if dy < 0.0 { 1.15 } else { 1.0 / 1.15 };
            let (wx, wy) = (da_z.width() as f64 / 2.0, da_z.height() as f64 / 2.0);
            zoom_model_around(
                &mut state_z.borrow_mut(),
                (wx, wy),
                factor,
                da_z.width() as f64,
                da_z.height() as f64,
            );
            da_z.queue_draw();
            glib::Propagation::Stop
        });
        drawing_area.add_controller(scroll);
    }

    // Drag to pan
    {
        let state_p = Rc::clone(&state);
        let da_p = drawing_area.clone();
        let drag_start: Rc<RefCell<Option<(f64, f64)>>> = Rc::new(RefCell::new(None));
        let pan_origin: Rc<RefCell<(f64, f64)>> = Rc::new(RefCell::new((0.0, 0.0)));

        let drag = gtk4::GestureDrag::new();
        {
            let ds = Rc::clone(&drag_start);
            let po = Rc::clone(&pan_origin);
            let st_c = Rc::clone(&state_p);
            drag.connect_drag_begin(move |_, x, y| {
                *ds.borrow_mut() = Some((x, y));
                let st = st_c.borrow();
                *po.borrow_mut() = (st.pan_x, st.pan_y);
            });
        }
        {
            let ds = Rc::clone(&drag_start);
            let po = Rc::clone(&pan_origin);
            let st_c = Rc::clone(&state_p);
            let da_c = da_p.clone();
            drag.connect_drag_update(move |_, ox, oy| {
                if let Some((sx, sy)) = *ds.borrow() {
                    let dx = ox - (sx - sx); // offset from begin
                    let dy = oy - (sy - sy);
                    let _ = (dx, dy); // suppress unused
                    let _ = (ox, oy);
                }
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

    // Timeline scrubber
    let timeline = Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 1.0);
    timeline.set_hexpand(true);
    timeline.set_draw_value(false);
    timeline.set_sensitive(false);
    timeline.set_margin_start(4);
    timeline.set_margin_end(4);
    left.append(&timeline);

    // Timeline scrubber handler
    {
        let state_tl = Rc::clone(&state);
        let da_tl = drawing_area.clone();
        let su_tl = Rc::clone(&slider_updating);
        timeline.connect_value_changed(move |scale| {
            if su_tl.get() {
                return;
            }
            let idx = scale.value() as usize;
            let mut st = state_tl.borrow_mut();
            let n = st.anim_frames.len();
            if n == 0 || idx >= n {
                return;
            }
            st.anim_index = idx;
            st.current_pixbuf = Some(st.anim_frames[idx].clone());
            drop(st);
            da_tl.queue_draw();
        });
    }

    // Status label
    let status = Label::new(Some("Select a product from the list →"));
    status.set_halign(gtk4::Align::Start);
    status.set_margin_start(4);
    status.set_margin_bottom(4);
    enable_status_copy(&status);
    left.append(&status);

    paned.set_start_child(Some(&left));

    // ── Right: search + favorites + collapsible product tree ─────────────────
    let right_box = GBox::new(Orientation::Vertical, 0);

    // ── Model type + sector selector row ──────────────────────────────────────
    let model_toolbar = GBox::new(Orientation::Horizontal, 4);
    model_toolbar.set_margin_start(4);
    model_toolbar.set_margin_end(4);
    model_toolbar.set_margin_top(4);
    model_toolbar.set_margin_bottom(2);

    const MODEL_IDS: &[&str] = &["sref", "gfs", "nam", "rap", "hrrr"];
    let model_combo = DropDown::from_strings(&["SREF", "GFS", "NAM", "RAP", "HRRR"]);
    model_combo.set_tooltip_text(Some("Model type"));
    // Don't set active here — we'll do it after the tree + handler are wired
    model_combo.set_hexpand(true);

    let sector_strings = StringList::new(&["CONUS"]);
    let sector_combo = DropDown::new(Some(sector_strings.clone()), gtk4::Expression::NONE);
    sector_combo.set_tooltip_text(Some("Area / Sector"));
    sector_combo.set_selected(0);
    sector_combo.set_visible(!matches!(initial_active_model, ActiveModel::Sref));

    model_toolbar.append(&model_combo);
    model_toolbar.append(&sector_combo);
    right_box.append(&model_toolbar);

    // Top bar: search entry + favorite toggle
    let tree_toolbar = GBox::new(Orientation::Horizontal, 4);
    tree_toolbar.set_margin_start(4);
    tree_toolbar.set_margin_end(4);
    tree_toolbar.set_margin_top(4);
    tree_toolbar.set_margin_bottom(2);

    let search_entry = SearchEntry::new();
    search_entry.set_hexpand(true);
    search_entry.set_placeholder_text(Some("Filter products…"));

    let fav_btn = ToggleButton::new();
    fav_btn.set_label("☆");
    fav_btn.set_tooltip_text(Some("Toggle favorite"));
    fav_btn.set_sensitive(false); // enabled when a product is selected

    tree_toolbar.append(&search_entry);
    tree_toolbar.append(&fav_btn);
    right_box.append(&tree_toolbar);

    let tree_scroll = ScrolledWindow::new();
    tree_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    tree_scroll.set_vexpand(true);
    tree_scroll.set_hexpand(false);
    tree_scroll.set_min_content_width(280);

    // TreeStore: col 0 = display label (String), col 1 = product_id (String),
    //            col 2 = visible (bool) — managed by search filter
    let store = TreeStore::new(&[glib::Type::STRING, glib::Type::STRING, glib::Type::BOOL]);

    // Helper: populate product rows into a parent iter
    let populate_products =
        |parent: Option<&gtk4::TreeIter>, products: &[meso_data::models::SrefProduct]| {
            for prod in products {
                let iter = store.append(parent);
                store.set(&iter, &[(0, &prod.label), (1, &prod.id), (2, &true)]);
            }
        };

    // ── Favorites category ────────────────────────────────────────────────────
    let fav_cat_iter: Rc<RefCell<Option<gtk4::TreeIter>>> = Rc::new(RefCell::new(None));
    let fi = store.append(None);
    store.set(&fi, &[(0, &"⭐ Favorites"), (1, &""), (2, &true)]);
    *fav_cat_iter.borrow_mut() = Some(fi.clone());
    // Populate saved favorites from config
    {
        let favs = shared_config.borrow().model_favorites.clone();
        let categories = models::sref_categories();
        for fav_id in &favs {
            // Look up label in product catalog
            'outer: for cat in categories {
                for prod in cat.products {
                    if prod.id == fav_id.as_str() {
                        let iter = store.append(Some(&fi));
                        store.set(&iter, &[(0, &prod.label), (1, &prod.id), (2, &true)]);
                        break 'outer;
                    }
                }
            }
        }
    }
    // Hide favorites section if empty
    let fav_has_children = store.iter_has_child(&fi);
    store.set(&fi, &[(2, &fav_has_children)]);

    // ── Full product catalog ──────────────────────────────────────────────────
    let categories = models::sref_categories();
    for cat in categories {
        let cat_iter = store.append(None);
        store.set(&cat_iter, &[(0, &cat.name), (1, &""), (2, &true)]);
        populate_products(Some(&cat_iter), cat.products);
    }

    // ── Filter model ──────────────────────────────────────────────────────────
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

    // Expand all categories by default
    tree_view.expand_all();

    // ── Search filtering ──────────────────────────────────────────────────────
    {
        let store_s = store.clone();
        let filter_s = filter.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_lowercase();
            // Walk categories
            if let Some(cat_iter) = store_s.iter_first() {
                loop {
                    let cat_id: String = store_s.get::<String>(&cat_iter, 1);
                    if cat_id.is_empty() {
                        // It's a category row — update product children visibility
                        let mut any_visible = false;
                        if let Some(child) = store_s.iter_children(Some(&cat_iter)) {
                            loop {
                                let label: String = store_s.get::<String>(&child, 0);
                                let visible =
                                    query.is_empty() || label.to_lowercase().contains(&query);
                                store_s.set(&child, &[(2, &visible)]);
                                if visible {
                                    any_visible = true;
                                }
                                if !store_s.iter_next(&child) {
                                    break;
                                }
                            }
                        }
                        // Favorites category: always visible if has children (don't hide on search)
                        let cat_label: String = store_s.get::<String>(&cat_iter, 0);
                        let is_fav_cat = cat_label.starts_with('⭐');
                        let cat_visible = if is_fav_cat {
                            store_s.iter_has_child(&cat_iter)
                        } else {
                            any_visible || query.is_empty()
                        };
                        store_s.set(&cat_iter, &[(2, &cat_visible)]);
                    }
                    if !store_s.iter_next(&cat_iter) {
                        break;
                    }
                }
            }
            filter_s.refilter();
        });
    }

    // Row activated — load selected product
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let ar_c = Rc::clone(&anim_running);
        let at_c = Rc::clone(&anim_timer);
        let tl_c = timeline.clone();
        let su_c = Rc::clone(&slider_updating);
        let anim_btn_c = anim_btn.clone();
        let refresh_btn_c = refresh_btn.clone();
        let fav_btn_c = fav_btn.clone();
        let cfg_c = Rc::clone(&shared_config);

        tree_view.connect_row_activated(move |tv, path, _col| {
            let model = tv.model().unwrap();
            let iter = model.iter(path).unwrap();
            let product_id: String = model.get::<String>(&iter, 1);
            let product_label: String = model.get::<String>(&iter, 0);
            if product_id.is_empty() {
                return;
            } // category row — ignore

            // Update fav button state
            let is_fav = cfg_c.borrow().model_favorites.contains(&product_id);
            fav_btn_c.set_active(is_fav);
            fav_btn_c.set_label(if is_fav { "★" } else { "☆" });
            fav_btn_c.set_sensitive(true);

            // Stop any running animation
            if ar_c.get() {
                ar_c.set(false);
                if let Some(id) = at_c.borrow_mut().take() {
                    id.remove();
                }
                anim_btn_c.set_label("▶ Animate");
            }
            // Reset animation state
            {
                let mut st = state_c.borrow_mut();
                st.anim_frames.clear();
                st.anim_timestamps.clear();
                st.anim_index = 0;
                st.current_product_id = product_id.clone();
                st.current_product_label = product_label.clone();
                st.zoom = 1.0;
                st.pan_x = 0.0;
                st.pan_y = 0.0;
            }
            su_c.set(true);
            tl_c.set_range(0.0, 1.0);
            tl_c.set_value(0.0);
            tl_c.set_sensitive(false);
            su_c.set(false);

            let active_model = state_c.borrow().active_model.clone();
            let sector = state_c.borrow().ncep_sector.clone();
            match active_model {
                ActiveModel::Sref => load_model_image(
                    product_id.clone(),
                    0,
                    Rc::clone(&state_c),
                    da_c.clone(),
                    st_c.clone(),
                    vec![refresh_btn_c.clone(), anim_btn_c.clone()],
                ),
                ActiveModel::Ncep(m) => load_ncep_image(
                    m,
                    sector,
                    product_id.clone(),
                    0,
                    Rc::clone(&state_c),
                    da_c.clone(),
                    st_c.clone(),
                    vec![refresh_btn_c.clone(), anim_btn_c.clone()],
                ),
            }
        });
    }

    // ── Favorite toggle button ─────────────────────────────────────────────
    {
        let state_f = Rc::clone(&state);
        let store_f = store.clone();
        let filter_f = filter.clone();
        let fav_iter_f = Rc::clone(&fav_cat_iter);
        let cfg_f = Rc::clone(&shared_config);
        let tv_f = tree_view.clone();
        let fav_btn_c = fav_btn.clone();

        fav_btn.connect_toggled(move |btn| {
            let product_id = state_f.borrow().current_product_id.clone();
            if product_id.is_empty() {
                return;
            }
            // Only apply favorites for SREF
            if state_f.borrow().active_model != ActiveModel::Sref {
                return;
            }
            let fi = match fav_iter_f.borrow().clone() {
                Some(fi) => fi,
                None => return,
            };
            let product_label = state_f.borrow().current_product_label.clone();

            let is_now_fav = btn.is_active();
            btn.set_label(if is_now_fav { "★" } else { "☆" });

            let mut cfg = cfg_f.borrow_mut();
            if is_now_fav {
                // Add to config favorites list
                if !cfg.model_favorites.contains(&product_id) {
                    cfg.model_favorites.push(product_id.clone());
                    // Add to favorites section in tree
                    let new_iter = store_f.append(Some(&fi));
                    store_f.set(
                        &new_iter,
                        &[
                            (0, &product_label.as_str()),
                            (1, &product_id.as_str()),
                            (2, &true),
                        ],
                    );
                    // Make favorites category visible
                    store_f.set(&fi, &[(2, &true)]);
                    filter_f.refilter();
                    tv_f.expand_all();
                }
            } else {
                // Remove from config
                cfg.model_favorites.retain(|id| id != &product_id);
                // Remove from favorites section in tree
                if let Some(child) = store_f.iter_children(Some(&fi)) {
                    loop {
                        let id: String = store_f.get::<String>(&child, 1);
                        if id == product_id {
                            store_f.remove(&child);
                            break;
                        }
                        if !store_f.iter_next(&child) {
                            break;
                        }
                    }
                }
                // Hide favorites category if now empty
                let has_favs = store_f.iter_has_child(&fi);
                store_f.set(&fi, &[(2, &has_favs)]);
                filter_f.refilter();
            }
            drop(cfg);
            let _ = fav_btn_c.is_active(); // suppress unused warning
        });
    }

    tree_scroll.set_child(Some(&tree_view));
    right_box.append(&tree_scroll);
    paned.set_end_child(Some(&right_box));

    outer.append(&paned);

    // ── Model type combo handler ───────────────────────────────────────────────
    {
        let store_m = store.clone();
        let filter_m = filter.clone();
        let tv_m = tree_view.clone();
        let state_m = Rc::clone(&state);
        let fav_iter_m = Rc::clone(&fav_cat_iter);
        let cfg_m = Rc::clone(&shared_config);
        let sector_combo_m = sector_combo.clone();
        let fav_btn_m = fav_btn.clone();
        let ar_m = Rc::clone(&anim_running);
        let at_m = Rc::clone(&anim_timer);
        let anim_btn_m = anim_btn.clone();

        let sector_strings_m = sector_strings.clone();

        model_combo.connect_selected_notify(move |combo| {
            let id = MODEL_IDS.get(combo.selected() as usize).copied().unwrap_or("sref");
            // Stop any running animation
            if ar_m.get() {
                ar_m.set(false);
                if let Some(src) = at_m.borrow_mut().take() {
                    src.remove();
                }
                anim_btn_m.set_label("▶ Animate");
            }

            let new_model = match id {
                "gfs"  => ActiveModel::Ncep(ncep::NcepModel::Gfs),
                "nam"  => ActiveModel::Ncep(ncep::NcepModel::Nam),
                "rap"  => ActiveModel::Ncep(ncep::NcepModel::Rap),
                "hrrr" => ActiveModel::Ncep(ncep::NcepModel::Hrrr),
                _      => ActiveModel::Sref,
            };

            // Update sector combo
            let sectors: Vec<&str> = match &new_model {
                ActiveModel::Sref => {
                    sector_combo_m.set_visible(false);
                    fav_btn_m.set_visible(true);
                    vec![]
                }
                ActiveModel::Ncep(m) => {
                    sector_combo_m.set_visible(true);
                    fav_btn_m.set_visible(false);
                    m.sectors().to_vec()
                }
            };
            if !sectors.is_empty() {
                // Rebuild sector combo via the shared StringList
                sector_strings_m.splice(0, sector_strings_m.n_items(), &sectors);
                let conus_pos = sectors.iter().position(|&s| s == "CONUS").unwrap_or(0);
                sector_combo_m.set_selected(conus_pos as u32);
            }

            // Update state
            {
                let mut st = state_m.borrow_mut();
                st.active_model = new_model.clone();
                st.ncep_sector = if sectors.is_empty() {
                    "CONUS".to_string()
                } else {
                    sectors[0].to_string()
                };
                st.ncep_run = String::new();
                st.current_product_id = String::new();
                st.current_product_label = String::new();
                st.current_pixbuf = None;
                st.anim_frames.clear();
                st.anim_timestamps.clear();
                st.anim_index = 0;
            }

            // Repopulate tree
            store_m.clear();
            match &new_model {
                ActiveModel::Sref => {
                    // Restore favorites
                    let fi = store_m.append(None);
                    store_m.set(&fi, &[(0, &"⭐ Favorites"), (1, &""), (2, &true)]);
                    let favs = cfg_m.borrow().model_favorites.clone();
                    let categories = models::sref_categories();
                    for fav_id in &favs {
                        'outer: for cat in categories {
                            for prod in cat.products {
                                if prod.id == fav_id.as_str() {
                                    let iter = store_m.append(Some(&fi));
                                    store_m.set(&iter, &[(0, &prod.label), (1, &prod.id), (2, &true)]);
                                    break 'outer;
                                }
                            }
                        }
                    }
                    let fav_vis = store_m.iter_has_child(&fi);
                    store_m.set(&fi, &[(2, &fav_vis)]);
                    *fav_iter_m.borrow_mut() = Some(fi);
                    // All SREF categories
                    for cat in models::sref_categories() {
                        let cat_iter = store_m.append(None);
                        store_m.set(&cat_iter, &[(0, &cat.name), (1, &""), (2, &true)]);
                        for prod in cat.products {
                            let iter = store_m.append(Some(&cat_iter));
                            store_m.set(&iter, &[(0, &prod.label), (1, &prod.id), (2, &true)]);
                        }
                    }
                }
                ActiveModel::Ncep(m) => {
                    *fav_iter_m.borrow_mut() = None;
                    for cat in m.categories() {
                        let cat_iter = store_m.append(None);
                        store_m.set(&cat_iter, &[(0, &cat.name), (1, &""), (2, &true)]);
                        for prod in cat.products {
                            let iter = store_m.append(Some(&cat_iter));
                            store_m.set(&iter, &[(0, &prod.label), (1, &prod.id), (2, &true)]);
                        }
                    }
                }
            }
            filter_m.refilter();
            tv_m.expand_all();
        });
    }

    // ── Sector combo handler ──────────────────────────────────────────────────
    {
        let state_s = Rc::clone(&state);
        let sector_strings_s = sector_strings.clone();
        sector_combo.connect_selected_notify(move |combo| {
            if let Some(obj) = sector_strings_s.string(combo.selected()) {
                let sector = obj.as_str().to_string();
                let mut st = state_s.borrow_mut();
                if st.ncep_sector != sector {
                    st.ncep_sector = sector;
                    st.ncep_run = String::new(); // invalidate cached run
                    st.anim_frames.clear();
                    st.anim_timestamps.clear();
                }
            }
        });
    }

    // Restore saved model type — fires connect_selected_notify which repopulates tree + sector combo
    if let Some(pos) = MODEL_IDS.iter().position(|&id| id == saved_model_type) {
        model_combo.set_selected(pos as u32);
    }
    // Restore saved sector after the model combo has repopulated the sector combo
    if saved_model_type != "sref" {
        let n = sector_strings.n_items();
        let saved = saved_sector.as_str();
        let pos = (0..n)
            .find(|&i| sector_strings.string(i).map_or(false, |s| s == saved))
            .unwrap_or(0);
        sector_combo.set_selected(pos);
    }

    // Save model type + sector when combos change (persist in config)
    {
        let cfg_persist = Rc::clone(&shared_config);
        let state_persist = Rc::clone(&state);
        model_combo.connect_selected_notify(move |_| {
            let st = state_persist.borrow();
            let type_str = match &st.active_model {
                ActiveModel::Sref     => "sref",
                ActiveModel::Ncep(m) => m.short(),
            };
            cfg_persist.borrow_mut().ncep_model_type = type_str.to_string();
        });
    }
    {
        let cfg_persist = Rc::clone(&shared_config);
        let state_persist = Rc::clone(&state);
        sector_combo.connect_selected_notify(move |_| {
            cfg_persist.borrow_mut().ncep_sector = state_persist.borrow().ncep_sector.clone();
        });
    }

    // ── Animate / pause button ────────────────────────────────────────────────
    {
        let state_a = Rc::clone(&state);
        let da_a = drawing_area.clone();
        let st_a = status.clone();
        let ar_a = Rc::clone(&anim_running);
        let at_a = Rc::clone(&anim_timer);
        let anim_btn_c = anim_btn.clone();
        let tl_a = timeline.clone();
        let su_a = Rc::clone(&slider_updating);
        let refresh_btn_a = refresh_btn.clone();

        anim_btn.connect_clicked(move |_| {
            if ar_a.get() {
                // Pause
                ar_a.set(false);
                if let Some(id) = at_a.borrow_mut().take() {
                    id.remove();
                }
                anim_btn_c.set_label("▶ Animate");
            } else if !state_a.borrow().anim_frames.is_empty() {
                // Resume
                ar_a.set(true);
                anim_btn_c.set_label("⏸ Pause");
                start_model_timer(
                    Rc::clone(&state_a),
                    da_a.clone(),
                    Rc::clone(&ar_a),
                    Rc::clone(&at_a),
                    tl_a.clone(),
                    Rc::clone(&su_a),
                );
             } else {
                // Fetch all frames
                let product_id = state_a.borrow().current_product_id.clone();
                if product_id.is_empty() {
                    return;
                }
                ar_a.set(true);
                anim_btn_c.set_label("⏸ Pause");
                let active_model = state_a.borrow().active_model.clone();
                let sector = state_a.borrow().ncep_sector.clone();
                match active_model {
                    ActiveModel::Sref => fetch_model_animation(
                        product_id,
                        Rc::clone(&state_a),
                        da_a.clone(),
                        st_a.clone(),
                        Rc::clone(&ar_a),
                        Rc::clone(&at_a),
                        tl_a.clone(),
                        Rc::clone(&su_a),
                        anim_btn_c.clone(),
                        refresh_btn_a.clone(),
                    ),
                    ActiveModel::Ncep(m) => fetch_ncep_animation(
                        m,
                        sector,
                        product_id,
                        Rc::clone(&state_a),
                        da_a.clone(),
                        st_a.clone(),
                        Rc::clone(&ar_a),
                        Rc::clone(&at_a),
                        tl_a.clone(),
                        Rc::clone(&su_a),
                        anim_btn_c.clone(),
                        refresh_btn_a.clone(),
                    ),
                }
            }
        });
    }

    // ── Refresh button ────────────────────────────────────────────────────────
    {
        let state_r = Rc::clone(&state);
        let da_r = drawing_area.clone();
        let st_r = status.clone();
        let ar_r = Rc::clone(&anim_running);
        let at_r = Rc::clone(&anim_timer);
        let tl_r = timeline.clone();
        let su_r = Rc::clone(&slider_updating);
        let anim_btn_r = anim_btn.clone();
        let refresh_btn_c = refresh_btn.clone();

        refresh_btn.connect_clicked(move |_| {
            // Stop animation, clear cached frames, reload F+000
            if ar_r.get() {
                ar_r.set(false);
                if let Some(id) = at_r.borrow_mut().take() {
                    id.remove();
                }
                anim_btn_r.set_label("▶ Animate");
            }
            {
                let mut st = state_r.borrow_mut();
                st.anim_frames.clear();
                st.anim_timestamps.clear();
                st.anim_index = 0;
            }
            su_r.set(true);
            tl_r.set_range(0.0, 1.0);
            tl_r.set_value(0.0);
            tl_r.set_sensitive(false);
            su_r.set(false);

            let product_id = state_r.borrow().current_product_id.clone();
            if !product_id.is_empty() {
                let active_model = state_r.borrow().active_model.clone();
                let sector = state_r.borrow().ncep_sector.clone();
                match active_model {
                    ActiveModel::Sref => load_model_image(
                        product_id,
                        0,
                        Rc::clone(&state_r),
                        da_r.clone(),
                        st_r.clone(),
                        vec![refresh_btn_c.clone(), anim_btn_r.clone()],
                    ),
                    ActiveModel::Ncep(m) => load_ncep_image(
                        m,
                        sector,
                        product_id,
                        0,
                        Rc::clone(&state_r),
                        da_r.clone(),
                        st_r.clone(),
                        vec![refresh_btn_c.clone(), anim_btn_r.clone()],
                    ),
                }
            }
        });
    }

    outer
}

// ── Zoom helper ───────────────────────────────────────────────────────────────

fn zoom_model_around(
    st: &mut ModelState,
    (wx, wy): (f64, f64),
    factor: f64,
    widget_w: f64,
    widget_h: f64,
) {
    if let Some(pb) = &st.current_pixbuf {
        let img_w = pb.width() as f64;
        let img_h = pb.height() as f64;
        let fit = (widget_w / img_w).min(widget_h / img_h);
        let total = fit * st.zoom;
        let img_x = (wx - widget_w / 2.0) / total + img_w / 2.0 + st.pan_x;
        let img_y = (wy - widget_h / 2.0) / total + img_h / 2.0 + st.pan_y;
        st.zoom = (st.zoom * factor).clamp(0.1, 20.0);
        let new_total = fit * st.zoom;
        st.pan_x = img_x - img_w / 2.0 - (wx - widget_w / 2.0) / new_total;
        st.pan_y = img_y - img_h / 2.0 - (wy - widget_h / 2.0) / new_total;
    } else {
        st.zoom = (st.zoom * factor).clamp(0.1, 20.0);
    }
}

// ── Image loading ─────────────────────────────────────────────────────────────

fn load_model_image(
    product_id: String,
    hour: u16,
    state: Rc<RefCell<ModelState>>,
    da: DrawingArea,
    status: Label,
    btns: Vec<Button>,
) {
    let label = state.borrow().current_product_label.clone();
    for b in &btns {
        b.set_sensitive(false);
    }
    status.set_text(&format!("Loading {label} F+{hour:03}..."));

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            let frame = models::fetch_sref_frame(&client, &product_id, hour).await?;
            let init_time = models::fetch_sref_init_time(&client).await;
            Ok::<_, anyhow::Error>((frame, init_time))
        },
        move |result| {
            for b in &btns {
                b.set_sensitive(true);
            }
            match result {
                Ok((bytes, init_time)) => {
                    if let Some(pb) = bytes_to_pixbuf(&bytes) {
                        let mut st = state.borrow_mut();
                        st.current_pixbuf = Some(pb);
                        if let Some(t) = init_time {
                            st.run_init_time = Some(t);
                        }
                        drop(st);
                        da.queue_draw();
                        status.set_text("Ready");
                    } else {
                        status.set_text("Failed to decode image");
                    }
                }
                Err(e) => status.set_text(&format!("Error: {e}")),
            }
        },
    );
}

// ── Animation ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn fetch_model_animation(
    product_id: String,
    state: Rc<RefCell<ModelState>>,
    da: DrawingArea,
    status: Label,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
    anim_btn: Button,
    refresh_btn: Button,
) {
    anim_btn.set_sensitive(false);
    refresh_btn.set_sensitive(false);

    let progress: runtime::ProgressSlot = Arc::new(Mutex::new(None));
    let stop_progress = runtime::progress_poller(Arc::clone(&progress), status.clone());

    let progress_c = Arc::clone(&progress);
    let pid = product_id.clone();
    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            // Fetch init time alongside frame data
            let init_time = models::fetch_sref_init_time(&client).await;
            let hours = models::sref_all_hours();
            let total = hours.len();
            let mut frames: Vec<(u16, Vec<u8>)> = Vec::new();
            for (i, &hour) in hours.iter().enumerate() {
                if let Ok(mut g) = progress_c.lock() {
                    *g = Some(format!("Fetching frame {}/{total} (F+{hour:03})…", i + 1));
                }
                let bytes = models::fetch_sref_frame(&client, &pid, hour).await?;
                frames.push((hour, bytes));
            }
            Ok::<_, anyhow::Error>((frames, init_time))
        },
        move |result| {
            stop_progress.set(true);
            anim_btn.set_sensitive(true);
            refresh_btn.set_sensitive(true);
            match result {
                Ok((frames, init_time)) => {
                    let pixbufs: Vec<gdk_pixbuf::Pixbuf> = frames
                        .iter()
                        .filter_map(|(_, b)| bytes_to_pixbuf(b))
                        .collect();
                    // Build local valid-time timestamps
                    let timestamps: Vec<String> = frames
                        .iter()
                        .map(|(h, _)| {
                            if let Some(init) = init_time {
                                let valid = init + chrono::Duration::hours(*h as i64);
                                let local = valid.with_timezone(&chrono::Local);
                                local.format("%m/%d %H:%M").to_string()
                            } else {
                                format!("F+{h:03}")
                            }
                        })
                        .collect();
                    if pixbufs.is_empty() {
                        status.set_text("Animation: no frames decoded");
                        running.set(false);
                        return;
                    }
                    let n = pixbufs.len();
                    {
                        let mut st = state.borrow_mut();
                        st.anim_frames = pixbufs;
                        st.anim_timestamps = timestamps;
                        st.anim_index = 0;
                        if let Some(t) = init_time {
                            st.run_init_time = Some(t);
                        }
                        st.current_pixbuf = Some(st.anim_frames[0].clone());
                    }
                    slider_updating.set(true);
                    timeline.set_range(0.0, (n - 1) as f64);
                    timeline.set_value(0.0);
                    timeline.set_sensitive(true);
                    slider_updating.set(false);
                    status.set_text(&format!("Animating {n} frames | 87h forecast"));
                    start_model_timer(
                        Rc::clone(&state),
                        da.clone(),
                        Rc::clone(&running),
                        Rc::clone(&timer),
                        timeline.clone(),
                        Rc::clone(&slider_updating),
                    );
                }
                Err(e) => {
                    status.set_text(&format!("Anim error: {e}"));
                    running.set(false);
                }
            }
        },
    );
}

fn start_model_timer(
    state: Rc<RefCell<ModelState>>,
    da: DrawingArea,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
) {
    let id = glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        if !running.get() {
            return glib::ControlFlow::Break;
        }
        let mut st = state.borrow_mut();
        if st.anim_frames.is_empty() {
            return glib::ControlFlow::Break;
        }
        st.anim_index = (st.anim_index + 1) % st.anim_frames.len();
        let i = st.anim_index;
        st.current_pixbuf = Some(st.anim_frames[i].clone());
        drop(st);
        slider_updating.set(true);
        timeline.set_value(i as f64);
        slider_updating.set(false);
        da.queue_draw();
        glib::ControlFlow::Continue
    });
    *timer.borrow_mut() = Some(id);
}

fn parse_ncep_run_time(run: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let digits: String = run.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 10 {
        return None;
    }
    let run10 = &digits[..10];
    let date = chrono::NaiveDate::parse_from_str(&run10[..8], "%Y%m%d").ok()?;
    let hour: u32 = run10[8..10].parse().ok()?;
    let naive = date.and_hms_opt(hour, 0, 0)?;
    Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        naive,
        chrono::Utc,
    ))
}

fn bytes_to_pixbuf(bytes: &[u8]) -> Option<gdk_pixbuf::Pixbuf> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    loader.pixbuf()
}

// ── NCEP model image loading ──────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn load_ncep_image(
    model: ncep::NcepModel,
    sector: String,
    product_id: String,
    hour: u16,
    state: Rc<RefCell<ModelState>>,
    da: DrawingArea,
    status: Label,
    btns: Vec<Button>,
) {
    for b in &btns {
        b.set_sensitive(false);
    }
    status.set_text(&format!("Loading {} {} F+{hour:03}...", model.label(), &product_id));

    // Grab cached run time (may be empty)
    let cached_run = state.borrow().ncep_run.clone();

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            let run = if cached_run.is_empty() {
                ncep::fetch_latest_run(&client, &model, &sector, &product_id).await?
            } else {
                cached_run
            };
            let bytes = ncep::fetch_frame(&client, &model, &run, &sector, &product_id, hour).await?;
            Ok::<_, anyhow::Error>((bytes, run))
        },
        move |result| {
            for b in &btns {
                b.set_sensitive(true);
            }
            match result {
                Ok((bytes, run)) => {
                    if let Some(pb) = bytes_to_pixbuf(&bytes) {
                        let mut st = state.borrow_mut();
                        st.ncep_run = run;
                        st.current_pixbuf = Some(pb);
                        drop(st);
                        da.queue_draw();
                        status.set_text("Ready");
                    } else {
                        status.set_text("Failed to decode image");
                    }
                }
                Err(e) => status.set_text(&format!("Error: {e}")),
            }
        },
    );
}

// ── NCEP animation ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn fetch_ncep_animation(
    model: ncep::NcepModel,
    sector: String,
    product_id: String,
    state: Rc<RefCell<ModelState>>,
    da: DrawingArea,
    status: Label,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
    anim_btn: Button,
    refresh_btn: Button,
) {
    anim_btn.set_sensitive(false);
    refresh_btn.set_sensitive(false);

    let progress: runtime::ProgressSlot = Arc::new(Mutex::new(None));
    let stop_progress = runtime::progress_poller(Arc::clone(&progress), status.clone());

    let cached_run = state.borrow().ncep_run.clone();
    let hours = model.forecast_hours();
    let model_label = model.label().to_string();

    let progress_c = Arc::clone(&progress);
    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            let run = if cached_run.is_empty() {
                ncep::fetch_latest_run(&client, &model, &sector, &product_id).await?
            } else {
                cached_run
            };
            let total = hours.len();
            let mut frames: Vec<(u16, Vec<u8>)> = Vec::new();
            let mut skipped: usize = 0;
            for (i, &hour) in hours.iter().enumerate() {
                if let Ok(mut g) = progress_c.lock() {
                    *g = Some(format!("Fetching frame {}/{total} (F+{hour:03})…", i + 1));
                }
                match ncep::fetch_frame(&client, &model, &run, &sector, &product_id, hour).await {
                    Ok(bytes) => frames.push((hour, bytes)),
                    Err(_) => skipped += 1,
                }
            }
            Ok::<_, anyhow::Error>((frames, run, skipped))
        },
        move |result| {
            stop_progress.set(true);
            anim_btn.set_sensitive(true);
            refresh_btn.set_sensitive(true);
            match result {
                Ok((frames, run, skipped)) => {
                    let pixbufs: Vec<gdk_pixbuf::Pixbuf> = frames
                        .iter()
                        .filter_map(|(_, b)| bytes_to_pixbuf(b))
                        .collect();
                    let run_init = parse_ncep_run_time(&run);
                    let timestamps: Vec<String> = frames
                        .iter()
                        .map(|(h, _)| {
                            if let Some(init) = run_init {
                                let valid = init + chrono::Duration::hours(*h as i64);
                                valid.with_timezone(&chrono::Local).format("%m/%d %H:%M").to_string()
                            } else {
                                format!("F+{h:03}")
                            }
                        })
                        .collect();
                    if pixbufs.is_empty() {
                        status.set_text("Animation: no frames available");
                        running.set(false);
                        return;
                    }
                    let n = pixbufs.len();
                    {
                        let mut st = state.borrow_mut();
                        st.ncep_run = run;
                        st.run_init_time = run_init;
                        st.anim_frames = pixbufs;
                        st.anim_timestamps = timestamps;
                        st.anim_index = 0;
                        st.current_pixbuf = Some(st.anim_frames[0].clone());
                    }
                    slider_updating.set(true);
                    timeline.set_range(0.0, (n - 1) as f64);
                    timeline.set_value(0.0);
                    timeline.set_sensitive(true);
                    slider_updating.set(false);
                    if skipped > 0 {
                        status.set_text(&format!("Animating {n} frames | {model_label} | skipped {skipped}"));
                    } else {
                        status.set_text(&format!("Animating {n} frames | {model_label}"));
                    }
                    start_model_timer(
                        Rc::clone(&state),
                        da.clone(),
                        Rc::clone(&running),
                        Rc::clone(&timer),
                        timeline.clone(),
                        Rc::clone(&slider_updating),
                    );
                }
                Err(e) => {
                    status.set_text(&format!("Anim error: {e}"));
                    running.set(false);
                }
            }
        },
    );
}
