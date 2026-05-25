/*
 * National weather products viewer pane.
 *
 * Layout (horizontal Paned):
 *   Left  — image viewer + toolbar
 *   Right — product tree (collapsible categories) + search
 *
 * Products: WPC Surface, WPC Forecast Maps, WPC QPF, NHC Tropical, Upper Air
 */

use glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, CellRendererText, DrawingArea, Label, Orientation, Paned, PolicyType,
    ScrolledWindow, SearchEntry, TreeModelFilter, TreeStore, TreeView, TreeViewColumn,
};

use std::cell::RefCell;
use std::rc::Rc;

use meso_data::national;

use crate::config::Config;
use crate::runtime;
use crate::ui::enable_status_copy;

// ── State ─────────────────────────────────────────────────────────────────────

struct NationalState {
    current_product_id: String,
    current_product_label: String,
    current_pixbuf: Option<gdk_pixbuf::Pixbuf>,
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
}

impl Default for NationalState {
    fn default() -> Self {
        NationalState {
            current_product_id: String::new(),
            current_product_label: String::new(),
            current_pixbuf: None,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        }
    }
}

// ── Public builder ────────────────────────────────────────────────────────────

pub fn build_national_pane(shared_config: Rc<RefCell<Config>>) -> GBox {
    let state = Rc::new(RefCell::new(NationalState::default()));

    let outer = GBox::new(Orientation::Vertical, 0);

    let paned = gtk4::Paned::new(Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_hexpand(true);

    let saved_pos = shared_config.borrow().national_pane_position;
    if saved_pos > 0 {
        paned.set_position(saved_pos);
    } else {
        let p_clone = paned.clone();
        paned.connect_realize(move |_| {
            let w = p_clone.allocated_width();
            if w > 10 {
                p_clone.set_position((w as f64 * 0.74) as i32);
            }
        });
    }
    {
        let cfg = Rc::clone(&shared_config);
        paned.connect_position_notify(move |p| {
            cfg.borrow_mut().national_pane_position = p.position();
        });
    }

    // ── Left: toolbar + image viewer ──────────────────────────────────────────
    let left = GBox::new(Orientation::Vertical, 0);

    let toolbar = GBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);

    let refresh_btn = Button::with_label("⟳ Refresh");
    refresh_btn.set_tooltip_text(Some("Reload this product"));
    toolbar.append(&refresh_btn);
    left.append(&toolbar);

    let drawing_area = DrawingArea::new();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);

    {
        let state_d = Rc::clone(&state);
        drawing_area.set_draw_func(move |_da, cr, w, h| {
            let st = state_d.borrow();
            let widget_w = w as f64;
            let widget_h = h as f64;

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
            } else if st.current_product_id.is_empty() {
                let msg = "Select a product from the list →";
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

    // Scroll to zoom
    {
        let state_z = Rc::clone(&state);
        let da_z = drawing_area.clone();
        let scroll =
            gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
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
                let fit =
                    (da_c.width() as f64 / pb_size.0).min(da_c.height() as f64 / pb_size.1);
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

    let status = Label::new(Some("Select a product from the list →"));
    status.set_halign(gtk4::Align::Start);
    status.set_margin_start(4);
    status.set_margin_bottom(4);
    enable_status_copy(&status);
    left.append(&status);

    paned.set_start_child(Some(&left));

    // ── Right: search + product tree ─────────────────────────────────────────
    let right_box = GBox::new(Orientation::Vertical, 0);

    let tree_toolbar = GBox::new(Orientation::Horizontal, 4);
    tree_toolbar.set_margin_start(4);
    tree_toolbar.set_margin_end(4);
    tree_toolbar.set_margin_top(4);
    tree_toolbar.set_margin_bottom(2);

    let search_entry = SearchEntry::new();
    search_entry.set_hexpand(true);
    search_entry.set_placeholder_text(Some("Filter products…"));
    tree_toolbar.append(&search_entry);
    right_box.append(&tree_toolbar);

    let tree_scroll = ScrolledWindow::new();
    tree_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    tree_scroll.set_vexpand(true);
    tree_scroll.set_hexpand(false);
    tree_scroll.set_min_content_width(260);

    // TreeStore: col 0 = label, col 1 = product_id, col 2 = visible
    let store = TreeStore::new(&[glib::Type::STRING, glib::Type::STRING, glib::Type::BOOL]);

    for cat in national::CATEGORIES {
        let cat_iter = store.append(None);
        store.set(&cat_iter, &[(0, &cat.name), (1, &""), (2, &true)]);
        for prod in cat.products {
            let iter = store.append(Some(&cat_iter));
            store.set(&iter, &[(0, &prod.label), (1, &prod.id), (2, &true)]);
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

    tree_view.expand_all();

    // Search
    {
        let store_s = store.clone();
        let filter_s = filter.clone();
        let tv_s = tree_view.clone();
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
                                if visible {
                                    any = true;
                                }
                                if !store_s.iter_next(&child) {
                                    break;
                                }
                            }
                        }
                        let cat_visible = any || query.is_empty();
                        store_s.set(&cat_iter, &[(2, &cat_visible)]);
                    }
                    if !store_s.iter_next(&cat_iter) {
                        break;
                    }
                }
            }
            filter_s.refilter();
            if query.is_empty() {
                tv_s.expand_all();
            }
        });
    }

    tree_scroll.set_child(Some(&tree_view));
    right_box.append(&tree_scroll);

    paned.set_end_child(Some(&right_box));
    outer.append(&paned);

    // ── Row activated → load product ─────────────────────────────────────────
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let refresh_btn_c = refresh_btn.clone();

        tree_view.connect_row_activated(move |tv, path, _col| {
            let model = tv.model().unwrap();
            let iter = model.iter(path).unwrap();
            let product_id: String = model.get::<String>(&iter, 1);
            let product_label: String = model.get::<String>(&iter, 0);
            if product_id.is_empty() {
                return;
            }

            {
                let mut st = state_c.borrow_mut();
                st.current_product_id = product_id.clone();
                st.current_product_label = product_label;
                st.current_pixbuf = None;
                st.zoom = 1.0;
                st.pan_x = 0.0;
                st.pan_y = 0.0;
            }
            da_c.queue_draw();

            load_product(
                product_id,
                Rc::clone(&state_c),
                da_c.clone(),
                st_c.clone(),
                vec![refresh_btn_c.clone()],
                false,
            );
        });
    }

    // ── Refresh ───────────────────────────────────────────────────────────────
    {
        let state_r = Rc::clone(&state);
        let da_r = drawing_area.clone();
        let st_r = status.clone();
        let refresh_btn_c = refresh_btn.clone();

        refresh_btn.connect_clicked(move |btn| {
            let product_id = state_r.borrow().current_product_id.clone();
            if product_id.is_empty() {
                return;
            }
            if let Some(url) = national::product_url(&product_id) {
                meso_data::cache::Cache::new("national").invalidate(url);
            }
            state_r.borrow_mut().current_pixbuf = None;
            da_r.queue_draw();

            load_product(
                product_id,
                Rc::clone(&state_r),
                da_r.clone(),
                st_r.clone(),
                vec![btn.clone()],
                true,
            );
        });
    }

    outer
}

// ── Zoom helper ───────────────────────────────────────────────────────────────

fn zoom_around(st: &mut NationalState, factor: f64, da: &DrawingArea) {
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

// ── Async image load ──────────────────────────────────────────────────────────

fn load_product(
    product_id: String,
    state: Rc<RefCell<NationalState>>,
    da: DrawingArea,
    status: Label,
    btns: Vec<Button>,
    bust_cache: bool,
) {
    for b in &btns {
        b.set_sensitive(false);
    }
    let label = state.borrow().current_product_label.clone();
    status.set_text(&format!("Loading {label}…"));

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            if bust_cache {
                // Already cleared before call; just fetch fresh
            }
            national::fetch_product(&client, &product_id).await
        },
        move |result| {
            for b in &btns {
                b.set_sensitive(true);
            }
            match result {
                Ok(bytes) => {
                    if let Some(pb) = bytes_to_pixbuf(&bytes) {
                        state.borrow_mut().current_pixbuf = Some(pb);
                        let label = state.borrow().current_product_label.clone();
                        da.queue_draw();
                        status.set_text(&format!("Loaded: {label}"));
                    } else {
                        status.set_text("Failed to decode image");
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
