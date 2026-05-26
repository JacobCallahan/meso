/*
 * Radar pane widget — fully interactive.
 *
 * Controls:
 *   Site combo      → update site + reload L3/L2
 *   Product combo   → update product + reload L3
 *   ⟳ Refresh       → reload current product (re-downloads, re-renders)
 *   L2 Ref          → fetch/render Level 2 reflectivity
 *   L2 Vel          → fetch/render Level 2 velocity
 *   Ref Palette     → select reflectivity color scheme
 *   Vel Palette     → select velocity color scheme
 *   Frames spin     → number of animation frames (2–60)
 *   ▶ Animate       → fetch N-frame loop (L3 or L2), cycle at 10 fps
 *   + / −           → zoom in/out (bottom-right overlay)
 * Mouse scroll      → zoom centered on cursor position
 * Click-drag        → pan
 */

use gdk_pixbuf::Pixbuf;
use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, DrawingArea, DropDown, Label, Orientation, Overlay, Popover, Scale,
    SpinButton, StringList,
};

use chrono::{DateTime, NaiveDateTime, Utc};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use meso_data::alerts::{fetch_active_alerts_by_state, Warning};
use meso_data::geo::latlon::LatLon;
use meso_data::geo::sites;
use meso_data::map_data::MapData;
use meso_data::radar::color_palette::PaletteRegistry;
use meso_data::radar::download::RadarDownloader;
use meso_data::radar::level2::{self, Level2Data};
use meso_data::radar::level3::{self, Level3Data};
use meso_data::radar::products::RadarProduct;
use meso_data::storm_tracks::fetch_storm_tracks;
use meso_data::updraft::{load_subscriptions, save_subscriptions};
use meso_render::cairo_render;
use meso_render::overlay::{
    build_storm_tracks_layer, build_warnings_layer, OverlayLayer, OverlaySet,
};
use meso_render::viewport::Viewport;

use crate::config::{Config, NamedLocation, RadarTrack, RadarTrackPoint};
use crate::radar_overlay_dialog::show_overlay_dialog;
use crate::runtime;
use crate::ui::enable_status_copy;

// ── State ─────────────────────────────────────────────────────────────────────

struct RadarPaneState {
    site_id: String,
    product: RadarProduct,
    viewport: Viewport,
    overlays: OverlaySet,
    warnings: Vec<Warning>,
    palette_registry: PaletteRegistry,
    palette_ref: String,
    palette_vel: String,
    map_data: Rc<MapData>,
    // Rendered display
    current_pixbuf: Option<Pixbuf>,
    // Timestamp for currently displayed frame (UTC formatted string)
    timestamp_str: Option<String>,
    // Decoded data cache (avoids re-downloading on zoom/pan)
    cached_l3: Option<Level3Data>,
    cached_l2: Option<(Level2Data, bool)>, // (data, is_velocity)
    // Animation
    anim_pixbufs: Vec<Pixbuf>,
    anim_timestamps: Vec<String>,
    anim_index: usize,
    // Decoded frames — kept for re-rendering on zoom/pan and palette change
    anim_l2_frames: Vec<(Level2Data, bool)>, // (data, is_velocity)
    anim_l3_frames: Vec<Level3Data>,
    // L2 multi-tilt support
    l2_tilt_idx: usize,
    l2_tilts: Vec<meso_data::radar::level2::TiltInfo>,
    cached_l2_bytes: Option<Vec<u8>>, // decompressed bytes for current single frame
}

impl RadarPaneState {
    fn new(cfg: &Config, map_data: Rc<MapData>, product_code: &str) -> Self {
        let site_center = sites::site_latlon(&cfg.radar_site).unwrap_or(LatLon {
            lat: 35.47,
            lon: -97.52,
        });
        // Restore saved viewport center if set, otherwise use site location.
        let center = if cfg.radar_center_lat != 0.0 || cfg.radar_center_lon != 0.0 {
            LatLon {
                lat: cfg.radar_center_lat,
                lon: cfg.radar_center_lon,
            }
        } else {
            site_center
        };
        let mut vp = Viewport::new(site_center, 800, 600);
        vp.center = center;
        vp.zoom = cfg.radar_zoom.max(0.1);
        let mut overlays = OverlaySet::new();
        overlays.rings_visible = cfg.radar_show_rings;
        RadarPaneState {
            site_id: cfg.radar_site.clone(),
            product: RadarProduct::from_code(product_code)
                .filter(|p| p.is_map_supported())
                .unwrap_or(RadarProduct::N0Q),
            viewport: vp,
            overlays,
            warnings: Vec::new(),
            palette_registry: PaletteRegistry::with_names(
                &cfg.radar_palette_ref,
                &cfg.radar_palette_vel,
            ),
            palette_ref: cfg.radar_palette_ref.clone(),
            palette_vel: cfg.radar_palette_vel.clone(),
            map_data,
            current_pixbuf: None,
            timestamp_str: None,
            cached_l3: None,
            cached_l2: None,
            anim_pixbufs: Vec::new(),
            anim_timestamps: Vec::new(),
            anim_index: 0,
            anim_l2_frames: Vec::new(),
            anim_l3_frames: Vec::new(),
            l2_tilt_idx: 0,
            l2_tilts: Vec::new(),
            cached_l2_bytes: None,
        }
    }

    /// Re-render from cached decoded data at the current viewport.
    /// Returns true if re-render succeeded.
    fn render_from_cache(&mut self) -> bool {
        let map = Some(self.map_data.as_ref());
        if let Some((l2, vel)) = &self.cached_l2 {
            let code: u16 = if *vel { 99 } else { 94 };
            let palette = self.palette_registry.for_product(code);
            if let Ok(img) =
                cairo_render::render_level2(l2, palette, &self.viewport, &self.overlays, *vel, map)
            {
                self.current_pixbuf = Some(rgba_to_pixbuf(&img));
                return true;
            }
        } else if let Some(l3) = &self.cached_l3 {
            let is_vel = self.product.is_velocity();
            let palette = self.palette_registry.for_product(l3.product_code);
            if let Ok(img) = cairo_render::render_level3(
                l3,
                palette,
                &self.viewport,
                &self.overlays,
                is_vel,
                map,
            ) {
                self.current_pixbuf = Some(rgba_to_pixbuf(&img));
                return true;
            }
        }
        false
    }

    fn clear_cache(&mut self) {
        self.cached_l3 = None;
        self.cached_l2 = None;
        self.cached_l2_bytes = None;
        self.current_pixbuf = None;
        self.timestamp_str = None;
        self.anim_pixbufs.clear();
        self.anim_timestamps.clear();
        self.anim_index = 0;
        self.anim_l2_frames.clear();
        self.anim_l3_frames.clear();
    }
}

// ── Public widget builder ─────────────────────────────────────────────────────

#[allow(clippy::type_complexity)]
pub fn build_radar_pane(
    shared_cfg: Rc<RefCell<Config>>,
) -> (GBox, Rc<dyn Fn(&str, Option<LatLon>)>) {
    let cfg_snapshot = shared_cfg.borrow().clone();
    let map_data = Rc::new(MapData::load());
    let left_state = Rc::new(RefCell::new(RadarPaneState::new(
        &cfg_snapshot,
        Rc::clone(&map_data),
        &cfg_snapshot.radar_product_left,
    )));
    let right_state = Rc::new(RefCell::new(RadarPaneState::new(
        &cfg_snapshot,
        Rc::clone(&map_data),
        &cfg_snapshot.radar_product_right,
    )));

    let pane_count = Rc::new(Cell::new(
        cfg_snapshot
            .radar_pane_count
            .clamp(1, 2)
            .max(if cfg_snapshot.radar_dual_pane { 2 } else { 1 }),
    ));
    let active_slot = Rc::new(Cell::new(0u8)); // 0 = left, 1 = right
    let shared_index = Rc::new(Cell::new(0usize));
    let anim_running = Rc::new(Cell::new(false));
    let anim_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let zoom_debounce: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let slider_updating: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let pending_center: Rc<Cell<Option<LatLon>>> = Rc::new(Cell::new(None));

    let vbox = GBox::new(Orientation::Vertical, 0);
    let toolbar = GBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_top(2);
    toolbar.set_margin_bottom(2);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);

    let sites_list = sites::all_sites();
    let current_site = left_state.borrow().site_id.clone();
    let site_combo = {
        let labels: Vec<String> = sites_list
            .iter()
            .map(|(id, name)| format!("{id} - {name}"))
            .collect();
        let combo = DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let active_idx = sites_list
            .iter()
            .position(|(id, _)| *id == current_site)
            .unwrap_or(0);
        combo.set_selected(active_idx as u32);
        combo
    };
    let site_ids: Vec<String> = sites_list.iter().map(|(id, _)| id.clone()).collect();
    site_combo.set_tooltip_text(Some("Select NEXRAD radar site"));
    toolbar.append(&site_combo);

    let active_label = Label::new(Some("L"));
    toolbar.append(&active_label);

    let group_combo = DropDown::from_strings(RadarProduct::PRODUCT_GROUPS);
    let prod_strings = StringList::new(&[]);
    let prod_codes: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(vec![]));
    let prod_combo = DropDown::new(Some(prod_strings.clone()), gtk4::Expression::NONE);
    let populate_products = {
        let prod_strings = prod_strings.clone();
        let prod_codes = Rc::clone(&prod_codes);
        move |group_combo: &DropDown, prod_combo: &DropDown, code: &str| {
            let selected = RadarProduct::from_code(code)
                .filter(|p| p.is_map_supported())
                .unwrap_or(RadarProduct::N0Q);
            let grp_pos = RadarProduct::PRODUCT_GROUPS
                .iter()
                .position(|&g| g == selected.group_name())
                .unwrap_or(0);
            group_combo.set_selected(grp_pos as u32);
            let products: Vec<RadarProduct> = RadarProduct::for_group(selected.group_name())
                .into_iter()
                .filter(|p| p.is_map_supported())
                .collect();
            let labels: Vec<&str> = products.iter().map(|p| p.label()).collect();
            prod_strings.splice(0, prod_strings.n_items(), &labels);
            *prod_codes.borrow_mut() = products.iter().map(|p| p.code()).collect();
            let code_pos = products
                .iter()
                .position(|p| p.code() == selected.code())
                .unwrap_or(0);
            prod_combo.set_selected(code_pos as u32);
        }
    };
    populate_products(
        &group_combo,
        &prod_combo,
        left_state.borrow().product.code(),
    );
    toolbar.append(&group_combo);
    toolbar.append(&prod_combo);
    group_combo.set_tooltip_text(Some("Select radar product category"));
    prod_combo.set_tooltip_text(Some("Select specific radar product"));

    // Tilt selector — shown only for L2 products
    let tilt_combo = DropDown::new(Some(StringList::new(&[])), gtk4::Expression::NONE);
    tilt_combo.set_tooltip_text(Some("Select elevation angle (L2 only)"));
    tilt_combo.set_visible(false);
    toolbar.append(&tilt_combo);

    let refresh_btn = Button::with_label("⟳");
    refresh_btn.set_tooltip_text(Some("Reload current radar product"));
    let anim_btn = Button::with_label("▶ Animate");
    anim_btn.set_tooltip_text(Some("Animate recent radar frames"));
    let pane_toggle_btn = Button::with_label(if pane_count.get() == 2 {
        "2▮"
    } else {
        "1▮"
    });
    pane_toggle_btn.set_tooltip_text(Some("Toggle 1/2 radar panes"));
    let overlay_btn = Button::with_label("⚙");
    overlay_btn.set_tooltip_text(Some("Radar display settings"));
    toolbar.append(&refresh_btn);

    // Subscribe button — ⚫ not subscribed, 🔵 subscribed
    let subscribe_btn = Button::with_label("⚫");
    subscribe_btn.set_tooltip_text(Some(
        "Subscribe to background caching for this station/product (meso-updraft)",
    ));
    toolbar.append(&subscribe_btn);

    let frames_label = Label::new(Some("Frames:"));
    toolbar.append(&frames_label);
    let frames_spin = SpinButton::with_range(2.0, 60.0, 1.0);
    frames_spin.set_value(shared_cfg.borrow().radar_anim_frames as f64);
    frames_spin.set_width_chars(3);
    toolbar.append(&frames_spin);
    toolbar.append(&anim_btn);
    toolbar.append(&pane_toggle_btn);
    toolbar.append(&overlay_btn);
    vbox.append(&toolbar);

    let left_da = DrawingArea::new();
    left_da.set_hexpand(true);
    left_da.set_vexpand(true);
    let right_da = DrawingArea::new();
    right_da.set_hexpand(true);
    right_da.set_vexpand(true);

    let make_overlay = |da: &DrawingArea| {
        let overlay = Overlay::new();
        overlay.set_child(Some(da));
        let zoom_box = GBox::new(Orientation::Horizontal, 2);
        zoom_box.set_halign(gtk4::Align::End);
        zoom_box.set_valign(gtk4::Align::End);
        zoom_box.set_margin_end(8);
        zoom_box.set_margin_bottom(8);
        zoom_box.add_css_class("linked");
        let zoom_out_btn = Button::with_label("−");
        let zoom_in_btn = Button::with_label("+");
        zoom_box.append(&zoom_out_btn);
        zoom_box.append(&zoom_in_btn);
        overlay.add_overlay(&zoom_box);
        (overlay, zoom_in_btn, zoom_out_btn)
    };
    let (left_overlay, left_zoom_in, left_zoom_out) = make_overlay(&left_da);
    let (right_overlay, right_zoom_in, right_zoom_out) = make_overlay(&right_da);
    right_overlay.set_visible(pane_count.get() == 2);
    left_overlay.add_css_class("radar-pane-active");
    right_overlay.add_css_class("radar-pane-inactive");

    let set_active_ui: Rc<dyn Fn(u8)> = {
        let active_slot = Rc::clone(&active_slot);
        let pane_count = Rc::clone(&pane_count);
        let active_label = active_label.clone();
        let group_combo = group_combo.clone();
        let prod_combo = prod_combo.clone();
        let prod_strings = prod_strings.clone();
        let prod_codes = Rc::clone(&prod_codes);
        let left_state = Rc::clone(&left_state);
        let right_state = Rc::clone(&right_state);
        let left_overlay = left_overlay.clone();
        let right_overlay = right_overlay.clone();
        Rc::new(move |slot_in: u8| {
            let slot = if pane_count.get() == 1 {
                0
            } else {
                slot_in.min(1)
            };
            active_slot.set(slot);
            active_label.set_text(if slot == 0 { "L" } else { "R" });

            left_overlay.remove_css_class("radar-pane-active");
            left_overlay.remove_css_class("radar-pane-inactive");
            right_overlay.remove_css_class("radar-pane-active");
            right_overlay.remove_css_class("radar-pane-inactive");
            if slot == 0 {
                left_overlay.add_css_class("radar-pane-active");
                right_overlay.add_css_class("radar-pane-inactive");
            } else {
                right_overlay.add_css_class("radar-pane-active");
                left_overlay.add_css_class("radar-pane-inactive");
            }

            let selected = if slot == 0 {
                left_state.borrow().product
            } else {
                right_state.borrow().product
            };
            let grp_pos = RadarProduct::PRODUCT_GROUPS
                .iter()
                .position(|&g| g == selected.group_name())
                .unwrap_or(0);
            group_combo.set_selected(grp_pos as u32);
            let products: Vec<RadarProduct> = RadarProduct::for_group(selected.group_name())
                .into_iter()
                .filter(|p| p.is_map_supported())
                .collect();
            let labels: Vec<&str> = products.iter().map(|p| p.label()).collect();
            prod_strings.splice(0, prod_strings.n_items(), &labels);
            *prod_codes.borrow_mut() = products.iter().map(|p| p.code()).collect();
            let code_pos = products
                .iter()
                .position(|p| p.code() == selected.code())
                .unwrap_or(0);
            prod_combo.set_selected(code_pos as u32);
        })
    };
    set_active_ui(0);

    let panes_row = GBox::new(Orientation::Horizontal, 4);
    panes_row.set_hexpand(true);
    panes_row.set_vexpand(true);
    panes_row.append(&left_overlay);
    panes_row.append(&right_overlay);
    vbox.append(&panes_row);

    let timeline = Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 1.0);
    timeline.set_hexpand(true);
    timeline.set_draw_value(false);
    timeline.set_sensitive(false);
    timeline.set_margin_start(4);
    timeline.set_margin_end(4);
    vbox.append(&timeline);

    let status = Label::new(Some("Ready"));
    status.set_halign(gtk4::Align::Start);
    enable_status_copy(&status);
    status.add_css_class("caption");
    vbox.append(&status);

    {
        let state_draw = Rc::clone(&left_state);
        let cfg_draw = Rc::clone(&shared_cfg);
        left_da.set_draw_func(move |_da, cr, w, h| {
            let st = state_draw.borrow();
            cr.set_source_rgb(0.0, 0.0, 0.0);
            let _ = cr.paint();
            if let Some(pb) = &st.current_pixbuf {
                cr.set_source_pixbuf(pb, 0.0, 0.0);
                let _ = cr.paint();
            } else {
                cr.set_source_rgb(0.4, 0.4, 0.4);
                cr.select_font_face(
                    "Sans",
                    gtk4::cairo::FontSlant::Normal,
                    gtk4::cairo::FontWeight::Normal,
                );
                cr.set_font_size(16.0);
                let text = "Loading radar...";
                let tx = (w as f64 - text.len() as f64 * 8.0) / 2.0;
                cr.move_to(tx, h as f64 / 2.0);
                let _ = cr.show_text(text);
            }
            if let Some(ts) = &st.timestamp_str {
                cr.select_font_face(
                    "Monospace",
                    gtk4::cairo::FontSlant::Normal,
                    gtk4::cairo::FontWeight::Bold,
                );
                cr.set_font_size(13.0);
                let x = 10.0_f64;
                let y = 22.0_f64;
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.85);
                for dx in [-1.0_f64, 0.0, 1.0] {
                    for dy in [-1.0_f64, 0.0, 1.0] {
                        if dx != 0.0 || dy != 0.0 {
                            cr.move_to(x + dx, y + dy);
                            let _ = cr.show_text(ts);
                        }
                    }
                }
                cr.set_source_rgb(1.0, 1.0, 0.8);
                cr.move_to(x, y);
                let _ = cr.show_text(ts);
            }
            draw_major_roads(
                cr,
                w,
                h,
                &st.viewport,
                st.map_data.as_ref(),
                &cfg_draw.borrow(),
            );
            draw_location_markers(cr, w, h, &st.viewport, &cfg_draw.borrow());
            draw_custom_tracks(cr, w, h, &st.viewport, &cfg_draw.borrow());
        });
    }
    {
        let state_draw = Rc::clone(&right_state);
        let cfg_draw = Rc::clone(&shared_cfg);
        right_da.set_draw_func(move |_da, cr, w, h| {
            let st = state_draw.borrow();
            cr.set_source_rgb(0.0, 0.0, 0.0);
            let _ = cr.paint();
            if let Some(pb) = &st.current_pixbuf {
                cr.set_source_pixbuf(pb, 0.0, 0.0);
                let _ = cr.paint();
            } else {
                cr.set_source_rgb(0.4, 0.4, 0.4);
                cr.select_font_face(
                    "Sans",
                    gtk4::cairo::FontSlant::Normal,
                    gtk4::cairo::FontWeight::Normal,
                );
                cr.set_font_size(16.0);
                let text = "Loading radar...";
                let tx = (w as f64 - text.len() as f64 * 8.0) / 2.0;
                cr.move_to(tx, h as f64 / 2.0);
                let _ = cr.show_text(text);
            }
            if let Some(ts) = &st.timestamp_str {
                cr.select_font_face(
                    "Monospace",
                    gtk4::cairo::FontSlant::Normal,
                    gtk4::cairo::FontWeight::Bold,
                );
                cr.set_font_size(13.0);
                let x = 10.0_f64;
                let y = 22.0_f64;
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.85);
                for dx in [-1.0_f64, 0.0, 1.0] {
                    for dy in [-1.0_f64, 0.0, 1.0] {
                        if dx != 0.0 || dy != 0.0 {
                            cr.move_to(x + dx, y + dy);
                            let _ = cr.show_text(ts);
                        }
                    }
                }
                cr.set_source_rgb(1.0, 1.0, 0.8);
                cr.move_to(x, y);
                let _ = cr.show_text(ts);
            }
            draw_major_roads(
                cr,
                w,
                h,
                &st.viewport,
                st.map_data.as_ref(),
                &cfg_draw.borrow(),
            );
            draw_location_markers(cr, w, h, &st.viewport, &cfg_draw.borrow());
            draw_custom_tracks(cr, w, h, &st.viewport, &cfg_draw.borrow());
        });
    }

    let apply_site_change = {
        let left_state = Rc::clone(&left_state);
        let right_state = Rc::clone(&right_state);
        let left_da = left_da.clone();
        let right_da = right_da.clone();
        let cfg = Rc::clone(&shared_cfg);
        let pane_count = Rc::clone(&pane_count);
        let anim_running = Rc::clone(&anim_running);
        let anim_timer = Rc::clone(&anim_timer);
        let anim_btn = anim_btn.clone();
        let status = status.clone();
        let tilt_combo = tilt_combo.clone();
        let site_ids = site_ids.clone();
        let pending_center = Rc::clone(&pending_center);
        let btns = vec![refresh_btn.clone(), anim_btn.clone()];

        Rc::new(move |selected_idx: usize| {
            let id = site_ids[selected_idx].clone();
            stop_animation(&anim_running, &anim_timer);
            anim_btn.set_label("▶ Animate");

            let center_opt = pending_center.take();
            let current_zoom = {
                let st = left_state.borrow();
                st.viewport.zoom
            };

            {
                let mut cfg = cfg.borrow_mut();
                cfg.radar_site = id.clone();
                if let Some(center) = center_opt {
                    cfg.radar_center_lat = center.lat;
                    cfg.radar_center_lon = center.lon;
                    cfg.radar_zoom = current_zoom;
                } else {
                    cfg.radar_zoom = 1.0;
                    cfg.radar_center_lat = 0.0;
                    cfg.radar_center_lon = 0.0;
                }
            }
            for st in [&left_state, &right_state] {
                let mut st = st.borrow_mut();
                st.site_id = id.clone();
                st.clear_cache();
                if let Some(loc) = sites::site_latlon(&id) {
                    let w = st.viewport.width;
                    let h = st.viewport.height;
                    let mut vp = Viewport::new(loc, w, h);
                    if let Some(center) = center_opt {
                        vp.center = center;
                        vp.zoom = current_zoom;
                    }
                    st.viewport = vp;
                }
            }
            trigger_load(
                Rc::clone(&left_state),
                left_da.clone(),
                status.clone(),
                btns.clone(),
                tilt_combo.clone(),
            );
            refresh_warnings(Rc::clone(&left_state), left_da.clone(), Rc::clone(&cfg));
            refresh_storm_tracks(Rc::clone(&left_state), left_da.clone(), Rc::clone(&cfg));
            if pane_count.get() == 2 {
                trigger_load(
                    Rc::clone(&right_state),
                    right_da.clone(),
                    status.clone(),
                    btns.clone(),
                    tilt_combo.clone(),
                );
                refresh_warnings(Rc::clone(&right_state), right_da.clone(), Rc::clone(&cfg));
                refresh_storm_tracks(Rc::clone(&right_state), right_da.clone(), Rc::clone(&cfg));
            }
            left_da.queue_draw();
            right_da.queue_draw();
        })
    };

    let change_site_fn: Rc<dyn Fn(&str, Option<LatLon>)> = {
        let site_combo = site_combo.clone();
        let site_ids = site_ids.clone();
        let pending_center = Rc::clone(&pending_center);
        let apply_site_change = Rc::clone(&apply_site_change);
        Rc::new(move |site_id: &str, latlon: Option<LatLon>| {
            pending_center.set(latlon);
            if let Some(pos) = site_ids.iter().position(|id| id == site_id) {
                if site_combo.selected() == pos as u32 {
                    apply_site_change(pos);
                } else {
                    site_combo.set_selected(pos as u32);
                }
            }
        })
    };

    let sync_zoom_pan = {
        let left_state = Rc::clone(&left_state);
        let right_state = Rc::clone(&right_state);
        let left_da = left_da.clone();
        let right_da = right_da.clone();
        let cfg = Rc::clone(&shared_cfg);
        let debounce = Rc::clone(&zoom_debounce);
        move |mutator: &dyn Fn(&mut Viewport)| {
            let mut has_anim = false;
            {
                let mut l = left_state.borrow_mut();
                mutator(&mut l.viewport);
                has_anim |= !l.anim_pixbufs.is_empty();
                if !has_anim {
                    l.render_from_cache();
                }
                let mut r = right_state.borrow_mut();
                r.viewport.center = l.viewport.center;
                r.viewport.zoom = l.viewport.zoom;
                has_anim |= !r.anim_pixbufs.is_empty();
                if !has_anim {
                    r.render_from_cache();
                }
                let mut cfg = cfg.borrow_mut();
                cfg.radar_zoom = l.viewport.zoom;
                cfg.radar_center_lat = l.viewport.center.lat;
                cfg.radar_center_lon = l.viewport.center.lon;
            }
            left_da.queue_draw();
            right_da.queue_draw();
            if has_anim {
                if let Some(id) = debounce.borrow_mut().take() {
                    id.remove();
                }
                let l = Rc::clone(&left_state);
                let r = Rc::clone(&right_state);
                let l_da = left_da.clone();
                let r_da = right_da.clone();
                let db = Rc::clone(&debounce);
                *debounce.borrow_mut() = Some(glib::timeout_add_local(
                    std::time::Duration::from_millis(300),
                    move || {
                        db.borrow_mut().take();
                        re_render_all_anim_frames_idle(Rc::clone(&l), l_da.clone());
                        re_render_all_anim_frames_idle(Rc::clone(&r), r_da.clone());
                        glib::ControlFlow::Break
                    },
                ));
            }
        }
    };

    {
        let left_state = Rc::clone(&left_state);
        left_da.connect_resize(move |_, w, h| {
            let mut l = left_state.borrow_mut();
            if l.viewport.width != w as u32 || l.viewport.height != h as u32 {
                l.viewport.width = w as u32;
                l.viewport.height = h as u32;
                l.render_from_cache();
            }
        });
    }
    {
        let right_state = Rc::clone(&right_state);
        right_da.connect_resize(move |_, w, h| {
            let mut r = right_state.borrow_mut();
            if r.viewport.width != w as u32 || r.viewport.height != h as u32 {
                r.viewport.width = w as u32;
                r.viewport.height = h as u32;
                r.render_from_cache();
            }
        });
    }

    let wire_nav_events =
        |da: &DrawingArea, is_left: bool, change_site_fn: Rc<dyn Fn(&str, Option<LatLon>)>| {
            let slot = if is_left { 0 } else { 1 };
            let leftclick = gtk4::GestureClick::new();
            leftclick.set_button(1);
            {
                let set_active = Rc::clone(&set_active_ui);
                leftclick.connect_pressed(move |_g, _n, _x, _y| {
                    set_active(slot);
                });
            }
            da.add_controller(leftclick);

            let mouse_pos = Rc::new(Cell::new((0.0f64, 0.0f64)));
            let motion = gtk4::EventControllerMotion::new();
            {
                let mp = Rc::clone(&mouse_pos);
                motion.connect_motion(move |_, x, y| {
                    mp.set((x, y));
                });
            }
            da.add_controller(motion);

            let scroll =
                gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
            {
                let mp = Rc::clone(&mouse_pos);
                let sync_fn = sync_zoom_pan.clone();
                let set_active = Rc::clone(&set_active_ui);
                scroll.connect_scroll(move |_, _dx, dy| {
                    set_active(slot);
                    let factor = if dy < 0.0 { 1.15 } else { 1.0 / 1.15 };
                    let (mx, my) = mp.get();
                    sync_fn(&|vp| {
                        vp.zoom_around_screen_point(mx, my, factor);
                    });
                    glib::Propagation::Stop
                });
            }
            da.add_controller(scroll);

            let drag = gtk4::GestureDrag::new();
            let last_offset = Rc::new(RefCell::new((0.0f64, 0.0f64)));
            {
                let sync_fn = sync_zoom_pan.clone();
                let last_off = Rc::clone(&last_offset);
                let set_active = Rc::clone(&set_active_ui);
                drag.connect_drag_update(move |_gesture, dx, dy| {
                    set_active(slot);
                    let (prev_dx, prev_dy) = *last_off.borrow();
                    let delta_x = dx - prev_dx;
                    let delta_y = dy - prev_dy;
                    *last_off.borrow_mut() = (dx, dy);
                    sync_fn(&|vp| {
                        vp.pan_pixels(delta_x, delta_y);
                    });
                });
            }
            {
                let last_off = Rc::clone(&last_offset);
                drag.connect_drag_end(move |_, _, _| {
                    *last_off.borrow_mut() = (0.0, 0.0);
                });
            }
            da.add_controller(drag);

            let rightclick = gtk4::GestureClick::new();
            rightclick.set_button(3);
            let set_active = Rc::clone(&set_active_ui);
            let status_rc = status.clone();
            let cfg_rc = Rc::clone(&shared_cfg);
            let shared_index_rc = Rc::clone(&shared_index);
            let left_da_rc = left_da.clone();
            let right_da_rc = right_da.clone();
            let menu_parent = da.clone();
            let pane_state = if is_left {
                Rc::clone(&left_state)
            } else {
                Rc::clone(&right_state)
            };
            rightclick.connect_pressed(move |gesture, _n, x, y| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                set_active(slot);
                let clicked_ll = {
                    let st = pane_state.borrow();
                    st.viewport.screen_to_latlon(x, y)
                };
                let frame_idx = shared_index_rc.get();
                let frame_time = {
                    let st = pane_state.borrow();
                    st.anim_timestamps
                        .get(frame_idx)
                        .cloned()
                        .or_else(|| st.timestamp_str.clone())
                };

                let popover = Popover::new();
                popover.set_has_arrow(true);
                popover.set_autohide(true);
                popover.set_parent(&menu_parent);
                let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                popover.set_pointing_to(Some(&rect));

                let menu = GBox::new(Orientation::Vertical, 2);
                menu.set_margin_top(6);
                menu.set_margin_bottom(6);
                menu.set_margin_start(6);
                menu.set_margin_end(6);

                let inspect_btn = Button::with_label("Inspect");
                let re_center_btn = Button::with_label("Re-center Here");
                let add_location_btn = Button::with_label("Add Location Here");
                let add_marker_btn = Button::with_label("Add Tracking Marker");
                let remove_marker_btn = Button::with_label("Remove Nearest Marker");
                let clear_track_btn = Button::with_label("Clear Active Track");

                {
                    let pane_state = Rc::clone(&pane_state);
                    let status = status_rc.clone();
                    let clicked = clicked_ll;
                    let pop = popover.clone();
                    inspect_btn.connect_clicked(move |_| {
                        let st = pane_state.borrow();
                        status.set_text(&build_inspect_message(&st, &clicked));
                        pop.popdown();
                    });
                }
                {
                    let cfg = Rc::clone(&cfg_rc);
                    let clicked = clicked_ll;
                    let pop = popover.clone();
                    let csf = Rc::clone(&change_site_fn);
                    re_center_btn.connect_clicked(move |_| {
                        let site_id = {
                            let mut cfg = cfg.borrow_mut();
                            cfg.active_location = String::new();
                            cfg.location_lat = clicked.lat;
                            cfg.location_lon = clicked.lon;
                            sites::nearest_site(&clicked, false).to_string()
                        };
                        csf(&site_id, Some(clicked));
                        pop.popdown();
                    });
                }
                {
                    let cfg = Rc::clone(&cfg_rc);
                    let status = status_rc.clone();
                    let clicked = clicked_ll;
                    let frame_time = frame_time.clone();
                    let left_da = left_da_rc.clone();
                    let right_da = right_da_rc.clone();
                    let pop = popover.clone();
                    add_marker_btn.connect_clicked(move |_| {
                        let point = RadarTrackPoint {
                            lat: clicked.lat,
                            lon: clicked.lon,
                            created_at: Utc::now().to_rfc3339(),
                            frame_index: frame_idx,
                            frame_time: frame_time.clone(),
                        };
                        let (track_name, point_count) = {
                            let mut cfg = cfg.borrow_mut();
                            let idx = append_track_point(&mut cfg, point);
                            let track = &cfg.radar_tracks[idx];
                            (track.name.clone(), track.points.len())
                        };
                        status.set_text(&format!(
                            "Added marker to {track_name} ({point_count} points)"
                        ));
                        left_da.queue_draw();
                        right_da.queue_draw();
                        pop.popdown();
                    });
                }
                {
                    let cfg = Rc::clone(&cfg_rc);
                    let status = status_rc.clone();
                    let clicked = clicked_ll;
                    let left_da = left_da_rc.clone();
                    let right_da = right_da_rc.clone();
                    let pop = popover.clone();
                    add_location_btn.connect_clicked(move |_| {
                        show_location_editor_dialog(
                            "New",
                            clicked.lat,
                            clicked.lon,
                            Rc::clone(&cfg),
                            status.clone(),
                            left_da.clone(),
                            right_da.clone(),
                        );
                        pop.popdown();
                    });
                }
                {
                    let cfg = Rc::clone(&cfg_rc);
                    let status = status_rc.clone();
                    let clicked = clicked_ll;
                    let left_da = left_da_rc.clone();
                    let right_da = right_da_rc.clone();
                    let pop = popover.clone();
                    remove_marker_btn.connect_clicked(move |_| {
                        let removed = {
                            let mut cfg = cfg.borrow_mut();
                            remove_nearest_track_point(&mut cfg, &clicked, 40.0)
                        };
                        if removed {
                            status.set_text("Removed nearest marker");
                        } else {
                            status.set_text("No marker near cursor");
                        }
                        left_da.queue_draw();
                        right_da.queue_draw();
                        pop.popdown();
                    });
                }
                {
                    let cfg = Rc::clone(&cfg_rc);
                    let status = status_rc.clone();
                    let left_da = left_da_rc.clone();
                    let right_da = right_da_rc.clone();
                    let pop = popover.clone();
                    clear_track_btn.connect_clicked(move |_| {
                        let cleared = {
                            let mut cfg = cfg.borrow_mut();
                            clear_active_track(&mut cfg)
                        };
                        if cleared {
                            status.set_text("Cleared active track");
                        } else {
                            status.set_text("No active track markers to clear");
                        }
                        left_da.queue_draw();
                        right_da.queue_draw();
                        pop.popdown();
                    });
                }

                for btn in [
                    &inspect_btn,
                    &re_center_btn,
                    &add_location_btn,
                    &add_marker_btn,
                    &remove_marker_btn,
                    &clear_track_btn,
                ] {
                    menu.append(btn);
                }
                popover.set_child(Some(&menu));
                popover.popup();
            });
            da.add_controller(rightclick);
        };
    wire_nav_events(&left_da, true, Rc::clone(&change_site_fn));
    wire_nav_events(&right_da, false, Rc::clone(&change_site_fn));

    {
        let sync_fn = sync_zoom_pan.clone();
        let set_active = Rc::clone(&set_active_ui);
        left_zoom_in.connect_clicked(move |_| {
            set_active(0);
            sync_fn(&|vp| {
                let cx = vp.width as f64 / 2.0;
                let cy = vp.height as f64 / 2.0;
                vp.zoom_around_screen_point(cx, cy, 1.5);
            });
        });
    }
    {
        let sync_fn = sync_zoom_pan.clone();
        let set_active = Rc::clone(&set_active_ui);
        left_zoom_out.connect_clicked(move |_| {
            set_active(0);
            sync_fn(&|vp| {
                let cx = vp.width as f64 / 2.0;
                let cy = vp.height as f64 / 2.0;
                vp.zoom_around_screen_point(cx, cy, 1.0 / 1.5);
            });
        });
    }
    {
        let sync_fn = sync_zoom_pan.clone();
        let set_active = Rc::clone(&set_active_ui);
        right_zoom_in.connect_clicked(move |_| {
            set_active(1);
            sync_fn(&|vp| {
                let cx = vp.width as f64 / 2.0;
                let cy = vp.height as f64 / 2.0;
                vp.zoom_around_screen_point(cx, cy, 1.5);
            });
        });
    }
    {
        let sync_fn = sync_zoom_pan.clone();
        let set_active = Rc::clone(&set_active_ui);
        right_zoom_out.connect_clicked(move |_| {
            set_active(1);
            sync_fn(&|vp| {
                let cx = vp.width as f64 / 2.0;
                let cy = vp.height as f64 / 2.0;
                vp.zoom_around_screen_point(cx, cy, 1.0 / 1.5);
            });
        });
    }

    {
        let pane_count_c = Rc::clone(&pane_count);
        let right_overlay_c = right_overlay.clone();
        let cfg_c = Rc::clone(&shared_cfg);
        let right_state_c = Rc::clone(&right_state);
        let right_da_c = right_da.clone();
        let left_state_c = Rc::clone(&left_state);
        let left_da_c = left_da.clone();
        let toggle_btn = pane_toggle_btn.clone();
        let set_active_ui = Rc::clone(&set_active_ui);
        let anim_running_c = Rc::clone(&anim_running);
        let anim_timer_c = Rc::clone(&anim_timer);
        pane_toggle_btn.connect_clicked(move |_| {
            let count = if pane_count_c.get() == 2 { 1 } else { 2 };
            pane_count_c.set(count);
            right_overlay_c.set_visible(count == 2);
            toggle_btn.set_label(if count == 2 { "2▮" } else { "1▮" });

            // Stop animation before switching panes
            stop_animation(&anim_running_c, &anim_timer_c);

            // Clear animation pixbuf caches when switching pane count so frames re-render at correct size
            {
                let mut left_st = left_state_c.borrow_mut();
                let has_anim =
                    !left_st.anim_l2_frames.is_empty() || !left_st.anim_l3_frames.is_empty();
                if has_anim {
                    left_st.anim_pixbufs.clear();
                    left_st.anim_index = 0;
                    left_st.current_pixbuf = None;
                }
            }
            {
                let mut right_st = right_state_c.borrow_mut();
                let has_anim =
                    !right_st.anim_l2_frames.is_empty() || !right_st.anim_l3_frames.is_empty();
                if has_anim {
                    right_st.anim_pixbufs.clear();
                    right_st.anim_index = 0;
                    right_st.current_pixbuf = None;
                }
            }

            let mut cfg = cfg_c.borrow_mut();
            cfg.radar_pane_count = count;
            cfg.radar_dual_pane = count == 2;
            drop(cfg);
            set_active_ui(if count == 2 { 1 } else { 0 });

            // Re-render animation frames at new viewport size if any exist
            {
                let left_st = left_state_c.borrow();
                if !left_st.anim_l2_frames.is_empty() || !left_st.anim_l3_frames.is_empty() {
                    drop(left_st);
                    re_render_all_anim_frames_idle(Rc::clone(&left_state_c), left_da_c.clone());
                }
            }
            {
                let right_st = right_state_c.borrow();
                if !right_st.anim_l2_frames.is_empty() || !right_st.anim_l3_frames.is_empty() {
                    drop(right_st);
                    re_render_all_anim_frames_idle(Rc::clone(&right_state_c), right_da_c.clone());
                }
            }

            if count == 2 {
                refresh_warnings(
                    Rc::clone(&right_state_c),
                    right_da_c.clone(),
                    Rc::clone(&cfg_c),
                );
                refresh_storm_tracks(
                    Rc::clone(&right_state_c),
                    right_da_c.clone(),
                    Rc::clone(&cfg_c),
                );
            }
            left_da_c.queue_draw();
            right_da_c.queue_draw();
        });
    }

    {
        let cfg_c = Rc::clone(&shared_cfg);
        frames_spin.connect_value_changed(move |spin| {
            cfg_c.borrow_mut().radar_anim_frames = spin.value() as u8;
        });
    }

    {
        {
            let apply_site_change = Rc::clone(&apply_site_change);
            site_combo.connect_selected_notify(move |combo| {
                apply_site_change(combo.selected() as usize);
            });
        }
    }

    {
        let combo = prod_combo.clone();
        let prod_strings_g = prod_strings.clone();
        let prod_codes_g = Rc::clone(&prod_codes);
        group_combo.connect_selected_notify(move |group_combo| {
            let group = RadarProduct::PRODUCT_GROUPS[group_combo.selected() as usize];
            let products: Vec<RadarProduct> = RadarProduct::for_group(group)
                .into_iter()
                .filter(|p| p.is_map_supported())
                .collect();
            let labels: Vec<&str> = products.iter().map(|p| p.label()).collect();
            prod_strings_g.splice(0, prod_strings_g.n_items(), &labels);
            *prod_codes_g.borrow_mut() = products.iter().map(|p| p.code()).collect();
            combo.set_selected(0);
        });
    }

    {
        let left_state_c = Rc::clone(&left_state);
        let right_state_c = Rc::clone(&right_state);
        let left_da_c = left_da.clone();
        let right_da_c = right_da.clone();
        let status_c = status.clone();
        let cfg_c = Rc::clone(&shared_cfg);
        let active_slot_c = Rc::clone(&active_slot);
        let pane_count_c = Rc::clone(&pane_count);
        let anim_running_c = Rc::clone(&anim_running);
        let anim_timer_c = Rc::clone(&anim_timer);
        let anim_btn_c = anim_btn.clone();
        let tilt_combo_c = tilt_combo.clone();
        let prod_codes_c = Rc::clone(&prod_codes);
        let btns = vec![refresh_btn.clone(), anim_btn.clone()];
        prod_combo.connect_selected_notify(move |combo| {
            let codes = prod_codes_c.borrow();
            if let Some(&code) = codes.get(combo.selected() as usize) {
                if let Some(prod) = RadarProduct::from_code(code).filter(|p| p.is_map_supported()) {
                    let slot = if pane_count_c.get() == 1 {
                        0
                    } else {
                        active_slot_c.get()
                    };
                    let current_product = if slot == 0 {
                        left_state_c.borrow().product
                    } else {
                        right_state_c.borrow().product
                    };
                    // Only stop animation and reload if the product actually changed
                    if current_product != prod {
                        stop_animation(&anim_running_c, &anim_timer_c);
                        anim_btn_c.set_label("▶ Animate");
                        if slot == 0 {
                            {
                                let mut cfg = cfg_c.borrow_mut();
                                cfg.radar_product_left = code.to_string();
                                cfg.radar_product = code.to_string();
                            }
                            {
                                let mut st = left_state_c.borrow_mut();
                                st.product = prod;
                                st.clear_cache();
                            }
                            trigger_load(
                                Rc::clone(&left_state_c),
                                left_da_c.clone(),
                                status_c.clone(),
                                btns.clone(),
                                tilt_combo_c.clone(),
                            );
                        } else {
                            cfg_c.borrow_mut().radar_product_right = code.to_string();
                            {
                                let mut st = right_state_c.borrow_mut();
                                st.product = prod;
                                st.clear_cache();
                            }
                            trigger_load(
                                Rc::clone(&right_state_c),
                                right_da_c.clone(),
                                status_c.clone(),
                                btns.clone(),
                                tilt_combo_c.clone(),
                            );
                        }
                    }
                }
            }
        });
    }

    // Subscribe button wiring — update label when station/product changes, toggle on click
    {
        let site_ids_sub = site_ids.clone();
        let prod_codes_sub = Rc::clone(&prod_codes);
        let site_sel = site_combo.clone();
        let prod_sel = prod_combo.clone();
        let btn = subscribe_btn.clone();
        let update_sub_btn = move || {
            let station = site_ids_sub
                .get(site_sel.selected() as usize)
                .cloned()
                .unwrap_or_default();
            let product = prod_codes_sub
                .borrow()
                .get(prod_sel.selected() as usize)
                .map(|&s| s.to_string())
                .unwrap_or_default();
            if !station.is_empty() && !product.is_empty() {
                let subs = load_subscriptions();
                btn.set_label(if subs.is_radar_subscribed(&station, &product) {
                    "🔵"
                } else {
                    "⚫"
                });
            }
        };
        update_sub_btn();

        // Re-check when station changes
        {
            let site_ids_2 = site_ids.clone();
            let prod_codes_2 = Rc::clone(&prod_codes);
            let prod_c2 = prod_combo.clone();
            let btn2 = subscribe_btn.clone();
            site_combo.connect_selected_notify(move |combo| {
                let station = site_ids_2
                    .get(combo.selected() as usize)
                    .cloned()
                    .unwrap_or_default();
                let product = prod_codes_2
                    .borrow()
                    .get(prod_c2.selected() as usize)
                    .map(|&s| s.to_string())
                    .unwrap_or_default();
                if !station.is_empty() && !product.is_empty() {
                    let subs = load_subscriptions();
                    btn2.set_label(if subs.is_radar_subscribed(&station, &product) {
                        "🔵"
                    } else {
                        "⚫"
                    });
                }
            });
        }

        // Re-check when product changes
        {
            let site_ids_3 = site_ids.clone();
            let prod_codes_3 = Rc::clone(&prod_codes);
            let site_c3 = site_combo.clone();
            let btn3 = subscribe_btn.clone();
            prod_combo.connect_selected_notify(move |combo| {
                let station = site_ids_3
                    .get(site_c3.selected() as usize)
                    .cloned()
                    .unwrap_or_default();
                let product = prod_codes_3
                    .borrow()
                    .get(combo.selected() as usize)
                    .map(|&s| s.to_string())
                    .unwrap_or_default();
                if !station.is_empty() && !product.is_empty() {
                    let subs = load_subscriptions();
                    btn3.set_label(if subs.is_radar_subscribed(&station, &product) {
                        "🔵"
                    } else {
                        "⚫"
                    });
                }
            });
        }

        // Toggle on click
        {
            let site_ids_4 = site_ids.clone();
            let prod_codes_4 = Rc::clone(&prod_codes);
            let site_c4 = site_combo.clone();
            let prod_c4 = prod_combo.clone();
            let btn4 = subscribe_btn.clone();
            subscribe_btn.connect_clicked(move |_| {
                let station = site_ids_4
                    .get(site_c4.selected() as usize)
                    .cloned()
                    .unwrap_or_default();
                let product = prod_codes_4
                    .borrow()
                    .get(prod_c4.selected() as usize)
                    .map(|&s| s.to_string())
                    .unwrap_or_default();
                if station.is_empty() || product.is_empty() {
                    return;
                }
                let mut subs = load_subscriptions();
                let now_subscribed = subs.toggle_radar(&station, &product);
                let _ = save_subscriptions(&subs);
                btn4.set_label(if now_subscribed { "🔵" } else { "⚫" });
            });
        }
    }

    {
        let left_state_c = Rc::clone(&left_state);
        let right_state_c = Rc::clone(&right_state);
        let left_da_c = left_da.clone();
        let right_da_c = right_da.clone();
        let status_c = status.clone();
        let cfg_c = Rc::clone(&shared_cfg);
        let pane_count_c = Rc::clone(&pane_count);
        let tilt_combo_c = tilt_combo.clone();
        let btns_ref = vec![refresh_btn.clone(), anim_btn.clone()];
        refresh_btn.connect_clicked(move |_| {
            left_state_c.borrow_mut().clear_cache();
            right_state_c.borrow_mut().clear_cache();
            trigger_load(
                Rc::clone(&left_state_c),
                left_da_c.clone(),
                status_c.clone(),
                btns_ref.clone(),
                tilt_combo_c.clone(),
            );
            refresh_warnings(
                Rc::clone(&left_state_c),
                left_da_c.clone(),
                Rc::clone(&cfg_c),
            );
            refresh_storm_tracks(
                Rc::clone(&left_state_c),
                left_da_c.clone(),
                Rc::clone(&cfg_c),
            );
            if pane_count_c.get() == 2 {
                trigger_load(
                    Rc::clone(&right_state_c),
                    right_da_c.clone(),
                    status_c.clone(),
                    btns_ref.clone(),
                    tilt_combo_c.clone(),
                );
                refresh_warnings(
                    Rc::clone(&right_state_c),
                    right_da_c.clone(),
                    Rc::clone(&cfg_c),
                );
                refresh_storm_tracks(
                    Rc::clone(&right_state_c),
                    right_da_c.clone(),
                    Rc::clone(&cfg_c),
                );
            }
        });
    }

    // Tilt selector handler — re-decode cached L2 bytes at the chosen tilt
    {
        let left_state_tc = Rc::clone(&left_state);
        let right_state_tc = Rc::clone(&right_state);
        let left_da_tc = left_da.clone();
        let right_da_tc = right_da.clone();
        let status_tc = status.clone();
        let active_slot_tc = Rc::clone(&active_slot);
        let pane_count_tc = Rc::clone(&pane_count);
        tilt_combo.connect_selected_notify(move |combo| {
            let sel = combo.selected() as usize;
            let slot = if pane_count_tc.get() == 1 {
                0
            } else {
                active_slot_tc.get()
            };
            let (state_ref, da_ref) = if slot == 0 {
                (Rc::clone(&left_state_tc), left_da_tc.clone())
            } else {
                (Rc::clone(&right_state_tc), right_da_tc.clone())
            };

            let (tilt_idx, raw_bytes, velocity) = {
                let st = state_ref.borrow();
                let idx = sel.min(st.l2_tilts.len().saturating_sub(1));
                let bytes = st.cached_l2_bytes.clone();
                let vel = st.product == RadarProduct::L2Velocity;
                (idx, bytes, vel)
            };
            let raw = match raw_bytes {
                Some(b) => b,
                None => return,
            };
            {
                let mut st = state_ref.borrow_mut();
                st.l2_tilt_idx = tilt_idx;
            }
            match level2::decode(&raw, velocity, tilt_idx) {
                Ok(l2) => {
                    let img = {
                        let st = state_ref.borrow();
                        let code: u16 = if velocity { 99 } else { 94 };
                        let palette = st.palette_registry.for_product(code);
                        cairo_render::render_level2(
                            &l2,
                            palette,
                            &st.viewport,
                            &st.overlays,
                            velocity,
                            Some(st.map_data.as_ref()),
                        )
                    };
                    match img {
                        Ok(img) => {
                            let mut st = state_ref.borrow_mut();
                            st.cached_l2 = Some((l2.clone(), velocity));
                            st.timestamp_str = Some(l2.timestamp_str());
                            st.current_pixbuf = Some(rgba_to_pixbuf(&img));
                            drop(st);
                            da_ref.queue_draw();
                        }
                        Err(e) => status_tc.set_text(&format!("Render error: {e}")),
                    }
                }
                Err(e) => status_tc.set_text(&format!("Tilt decode error: {e}")),
            }
        });
    }

    {
        let cfg_ov = Rc::clone(&shared_cfg);
        let left_state_ov = Rc::clone(&left_state);
        let right_state_ov = Rc::clone(&right_state);
        let left_da_ov = left_da.clone();
        let right_da_ov = right_da.clone();
        overlay_btn.connect_clicked(move |btn| {
            let win = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
            if let Some(win) = win {
                let cfg2 = Rc::clone(&cfg_ov);
                let left_state2 = Rc::clone(&left_state_ov);
                let right_state2 = Rc::clone(&right_state_ov);
                let left_da2 = left_da_ov.clone();
                let right_da2 = right_da_ov.clone();
                show_overlay_dialog(&win, cfg2.clone(), move || {
                    let cfg = cfg2.borrow();
                    let ref_name = cfg.radar_palette_ref.clone();
                    let vel_name = cfg.radar_palette_vel.clone();
                    let left_has_anim = {
                        let mut st = left_state2.borrow_mut();
                        st.overlays.set_visible("warnings", cfg.radar_show_warnings);
                        st.overlays
                            .set_visible("storm_tracks", cfg.radar_show_storm_tracks);
                        st.overlays.rings_visible = cfg.radar_show_rings;
                        st.palette_ref = ref_name.clone();
                        st.palette_vel = vel_name.clone();
                        st.palette_registry = PaletteRegistry::with_names(&ref_name, &vel_name);
                        !st.anim_l2_frames.is_empty() || !st.anim_l3_frames.is_empty()
                    };
                    let right_has_anim = {
                        let mut st = right_state2.borrow_mut();
                        st.overlays.set_visible("warnings", cfg.radar_show_warnings);
                        st.overlays
                            .set_visible("storm_tracks", cfg.radar_show_storm_tracks);
                        st.overlays.rings_visible = cfg.radar_show_rings;
                        st.palette_ref = ref_name.clone();
                        st.palette_vel = vel_name.clone();
                        st.palette_registry = PaletteRegistry::with_names(&ref_name, &vel_name);
                        !st.anim_l2_frames.is_empty() || !st.anim_l3_frames.is_empty()
                    };
                    if left_has_anim {
                        re_render_all_anim_frames_idle(Rc::clone(&left_state2), left_da2.clone());
                    } else {
                        left_state2.borrow_mut().render_from_cache();
                        left_da2.queue_draw();
                    }
                    if right_has_anim {
                        re_render_all_anim_frames_idle(Rc::clone(&right_state2), right_da2.clone());
                    } else {
                        right_state2.borrow_mut().render_from_cache();
                        right_da2.queue_draw();
                    }
                });
            }
        });
    }

    {
        let left_state_c = Rc::clone(&left_state);
        let right_state_c = Rc::clone(&right_state);
        let left_da_c = left_da.clone();
        let right_da_c = right_da.clone();
        let pane_count_c = Rc::clone(&pane_count);
        let status_c = status.clone();
        let ar_c = Rc::clone(&anim_running);
        let at_c = Rc::clone(&anim_timer);
        let anim_btn_c = anim_btn.clone();
        let frames_s = frames_spin.clone();
        let timeline_c = timeline.clone();
        let su_c = Rc::clone(&slider_updating);
        let shared_index_c = Rc::clone(&shared_index);
        anim_btn.connect_clicked(move |_| {
            if ar_c.get() {
                stop_animation(&ar_c, &at_c);
                anim_btn_c.set_label("▶ Animate");
                return;
            }
            let active_states = if pane_count_c.get() == 2 {
                vec![
                    (Rc::clone(&left_state_c), left_da_c.clone()),
                    (Rc::clone(&right_state_c), right_da_c.clone()),
                ]
            } else {
                vec![(Rc::clone(&left_state_c), left_da_c.clone())]
            };
            let all_ready = active_states
                .iter()
                .all(|(s, _)| !s.borrow().anim_pixbufs.is_empty());
            if all_ready {
                ar_c.set(true);
                anim_btn_c.set_label("⏸ Pause");
                start_shared_timer(
                    active_states,
                    Rc::clone(&ar_c),
                    Rc::clone(&at_c),
                    timeline_c.clone(),
                    Rc::clone(&su_c),
                    Rc::clone(&shared_index_c),
                );
                return;
            }

            // Single-pane: use the original animation path with granular
            // fetch/decode/render progress updates.
            if pane_count_c.get() == 1 {
                let (state, da) = active_states[0].clone();
                let frame_count = frames_s.value() as usize;
                start_animation(
                    state,
                    da,
                    status_c.clone(),
                    Rc::clone(&ar_c),
                    Rc::clone(&at_c),
                    frame_count,
                    anim_btn_c.clone(),
                    timeline_c.clone(),
                    Rc::clone(&su_c),
                );
                return;
            }

            anim_btn_c.set_sensitive(false);
            status_c.set_text("Fetching animation...");
            let pending = Rc::new(Cell::new(active_states.len()));
            let min_frames = Rc::new(Cell::new(usize::MAX));
            let failed = Rc::new(Cell::new(false));
            for (state, da) in active_states.clone() {
                let pending_c = Rc::clone(&pending);
                let min_frames_c = Rc::clone(&min_frames);
                let failed_c = Rc::clone(&failed);
                let status_done = status_c.clone();
                let anim_btn_done = anim_btn_c.clone();
                let timeline_done = timeline_c.clone();
                let su_done = Rc::clone(&su_c);
                let ar_done = Rc::clone(&ar_c);
                let at_done = Rc::clone(&at_c);
                let shared_index_done = Rc::clone(&shared_index_c);
                let states_done = active_states.clone();
                let frame_count = frames_s.value() as usize;
                load_animation_frames(
                    Rc::clone(&state),
                    da.clone(),
                    frame_count,
                    status_c.clone(),
                    move |result| {
                        match result {
                            Ok(n) => min_frames_c.set(min_frames_c.get().min(n)),
                            Err(e) => {
                                failed_c.set(true);
                                status_done.set_text(&format!("Anim error: {e}"));
                            }
                        }
                        let left = pending_c.get().saturating_sub(1);
                        pending_c.set(left);
                        if left == 0 {
                            anim_btn_done.set_sensitive(true);
                            if failed_c.get()
                                || min_frames_c.get() == 0
                                || min_frames_c.get() == usize::MAX
                            {
                                ar_done.set(false);
                                anim_btn_done.set_label("▶ Animate");
                                return;
                            }
                            let n = min_frames_c.get();
                            for (st, draw) in &states_done {
                                let mut st = st.borrow_mut();
                                st.anim_pixbufs.truncate(n);
                                st.anim_timestamps.truncate(n);
                                st.anim_l2_frames.truncate(n);
                                st.anim_l3_frames.truncate(n);
                                st.anim_index = 0;
                                st.current_pixbuf = st.anim_pixbufs.first().cloned();
                                st.timestamp_str = st.anim_timestamps.first().cloned();
                                draw.queue_draw();
                            }
                            shared_index_done.set(0);
                            su_done.set(true);
                            timeline_done.set_range(0.0, (n - 1) as f64);
                            timeline_done.set_value(0.0);
                            timeline_done.set_sensitive(true);
                            su_done.set(false);
                            status_done.set_text(&format!("Animating {n} frames"));
                            ar_done.set(true);
                            anim_btn_done.set_label("⏸ Pause");
                            start_shared_timer(
                                states_done.clone(),
                                Rc::clone(&ar_done),
                                Rc::clone(&at_done),
                                timeline_done.clone(),
                                Rc::clone(&su_done),
                                Rc::clone(&shared_index_done),
                            );
                        }
                    },
                );
            }
        });
    }

    {
        let left_state_tl = Rc::clone(&left_state);
        let right_state_tl = Rc::clone(&right_state);
        let left_da_tl = left_da.clone();
        let right_da_tl = right_da.clone();
        let pane_count_tl = Rc::clone(&pane_count);
        let su_tl = Rc::clone(&slider_updating);
        let shared_index_tl = Rc::clone(&shared_index);
        let anim_running_tl = Rc::clone(&anim_running);
        let anim_timer_tl = Rc::clone(&anim_timer);
        let anim_btn_tl = anim_btn.clone();
        timeline.connect_value_changed(move |scale| {
            if su_tl.get() {
                return;
            }
            // User scrub takes control: pause active animation loop.
            if anim_running_tl.get() {
                stop_animation(&anim_running_tl, &anim_timer_tl);
                anim_btn_tl.set_label("▶ Animate");
                anim_btn_tl.set_sensitive(true);
            }
            let idx = scale.value() as usize;
            shared_index_tl.set(idx);
            {
                let mut st = left_state_tl.borrow_mut();
                if idx < st.anim_pixbufs.len() {
                    st.anim_index = idx;
                    st.current_pixbuf = Some(st.anim_pixbufs[idx].clone());
                    st.timestamp_str = st.anim_timestamps.get(idx).cloned();
                }
            }
            left_da_tl.queue_draw();
            if pane_count_tl.get() == 2 {
                let mut st = right_state_tl.borrow_mut();
                if idx < st.anim_pixbufs.len() {
                    st.anim_index = idx;
                    st.current_pixbuf = Some(st.anim_pixbufs[idx].clone());
                    st.timestamp_str = st.anim_timestamps.get(idx).cloned();
                }
                drop(st);
                right_da_tl.queue_draw();
            }
        });
    }

    trigger_load(
        Rc::clone(&left_state),
        left_da.clone(),
        status.clone(),
        vec![refresh_btn.clone(), anim_btn.clone()],
        tilt_combo.clone(),
    );
    refresh_warnings(
        Rc::clone(&left_state),
        left_da.clone(),
        Rc::clone(&shared_cfg),
    );
    refresh_storm_tracks(
        Rc::clone(&left_state),
        left_da.clone(),
        Rc::clone(&shared_cfg),
    );
    if pane_count.get() == 2 {
        trigger_load(
            Rc::clone(&right_state),
            right_da.clone(),
            status.clone(),
            vec![refresh_btn.clone(), anim_btn.clone()],
            tilt_combo.clone(),
        );
        refresh_warnings(
            Rc::clone(&right_state),
            right_da.clone(),
            Rc::clone(&shared_cfg),
        );
        refresh_storm_tracks(
            Rc::clone(&right_state),
            right_da.clone(),
            Rc::clone(&shared_cfg),
        );
    }

    {
        let left_state_ar = Rc::clone(&left_state);
        let right_state_ar = Rc::clone(&right_state);
        let left_da_ar = left_da.clone();
        let right_da_ar = right_da.clone();
        let st_ar = status.clone();
        let ar_ar = Rc::clone(&anim_running);
        let cfg_ar = Rc::clone(&shared_cfg);
        let pane_count_ar = Rc::clone(&pane_count);
        let tilt_combo_ar = tilt_combo.clone();
        let btns_ar = vec![refresh_btn.clone(), anim_btn.clone()];
        glib::timeout_add_local(std::time::Duration::from_secs(90), move || {
            if !ar_ar.get() {
                trigger_load(
                    Rc::clone(&left_state_ar),
                    left_da_ar.clone(),
                    st_ar.clone(),
                    btns_ar.clone(),
                    tilt_combo_ar.clone(),
                );
                refresh_warnings(
                    Rc::clone(&left_state_ar),
                    left_da_ar.clone(),
                    Rc::clone(&cfg_ar),
                );
                refresh_storm_tracks(
                    Rc::clone(&left_state_ar),
                    left_da_ar.clone(),
                    Rc::clone(&cfg_ar),
                );
                if pane_count_ar.get() == 2 {
                    trigger_load(
                        Rc::clone(&right_state_ar),
                        right_da_ar.clone(),
                        st_ar.clone(),
                        btns_ar.clone(),
                        tilt_combo_ar.clone(),
                    );
                    refresh_warnings(
                        Rc::clone(&right_state_ar),
                        right_da_ar.clone(),
                        Rc::clone(&cfg_ar),
                    );
                    refresh_storm_tracks(
                        Rc::clone(&right_state_ar),
                        right_da_ar.clone(),
                        Rc::clone(&cfg_ar),
                    );
                }
            }
            glib::ControlFlow::Continue
        });
    }

    (vbox, change_site_fn)
}

// ── Animation helpers ─────────────────────────────────────────────────────────

fn stop_animation(running: &Rc<Cell<bool>>, timer: &Rc<RefCell<Option<glib::SourceId>>>) {
    running.set(false);
    if let Some(id) = timer.borrow_mut().take() {
        id.remove();
    }
}

fn start_shared_timer(
    states: Vec<(Rc<RefCell<RadarPaneState>>, DrawingArea)>,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
    shared_index: Rc<Cell<usize>>,
) {
    if let Some(id) = timer.borrow_mut().take() {
        id.remove();
    }
    let id = glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if !running.get() {
            return glib::ControlFlow::Break;
        }
        let frame_count = states
            .iter()
            .map(|(s, _)| s.borrow().anim_pixbufs.len())
            .filter(|n| *n > 0)
            .min()
            .unwrap_or(0);
        if frame_count == 0 {
            return glib::ControlFlow::Break;
        }
        let idx = (shared_index.get() + 1) % frame_count;
        shared_index.set(idx);
        for (state, da) in &states {
            let mut st = state.borrow_mut();
            st.anim_index = idx;
            if idx < st.anim_pixbufs.len() {
                st.current_pixbuf = Some(st.anim_pixbufs[idx].clone());
                st.timestamp_str = st.anim_timestamps.get(idx).cloned();
            }
            drop(st);
            da.queue_draw();
        }
        slider_updating.set(true);
        timeline.set_value(idx as f64);
        slider_updating.set(false);
        glib::ControlFlow::Continue
    });
    *timer.borrow_mut() = Some(id);
}

fn load_animation_frames<F>(
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    frame_count: usize,
    status: Label,
    on_done: F,
) where
    F: Fn(Result<usize, String>) + 'static,
{
    let on_done: Rc<dyn Fn(Result<usize, String>)> = Rc::new(on_done);
    let product = state.borrow().product;
    let site_id = state.borrow().site_id.clone();
    let progress: runtime::ProgressSlot = Arc::new(Mutex::new(None));
    let stop_progress = runtime::progress_poller(Arc::clone(&progress), status.clone());
    if product.is_level2() {
        let velocity = product == RadarProduct::L2Velocity;
        let progress_c = Arc::clone(&progress);
        runtime::spawn(
            async move {
                if let Ok(mut g) = progress_c.lock() {
                    *g = Some("Reading Level 2 frame inventory...".to_string());
                }
                let client = meso_data::http::wx_client();
                let dl = RadarDownloader::new(client);
                let l2_product = if velocity {
                    RadarProduct::L2Velocity
                } else {
                    RadarProduct::L2Reflectivity
                };
                let filenames = dl
                    .level2_filenames_for_animation(&site_id, frame_count)
                    .await?;
                let base = RadarDownloader::level2_dir_url(&site_id);
                let mut frames = Vec::new();
                let total = filenames.len();
                for (i, fname) in filenames.iter().enumerate() {
                    if let Ok(mut g) = progress_c.lock() {
                        *g = Some(format!("Fetching/decompressing frame {}/{}", i + 1, total));
                    }
                    let url = format!("{base}{fname}");
                    frames.push(
                        dl.fetch_level2_decompressed(&site_id, &l2_product, &url)
                            .await?,
                    );
                }
                Ok::<_, anyhow::Error>((frames, velocity))
            },
            move |result| match result {
                Ok((decomp_frames, vel)) => {
                    stop_progress.set(true);
                    status.set_text(&format!(
                        "Constructing animation from {} frames...",
                        decomp_frames.len()
                    ));
                    let mut pixbufs = Vec::new();
                    let mut timestamps = Vec::new();
                    let mut decoded = Vec::new();
                    for decompressed in decomp_frames {
                        let tilt_idx = state.borrow().l2_tilt_idx;
                        let st = state.borrow();
                        if let Ok(l2) = level2::decode(&decompressed, vel, tilt_idx) {
                            let code: u16 = if vel { 99 } else { 94 };
                            let palette = st.palette_registry.for_product(code);
                            if let Ok(img) = cairo_render::render_level2(
                                &l2,
                                palette,
                                &st.viewport,
                                &st.overlays,
                                vel,
                                Some(st.map_data.as_ref()),
                            ) {
                                timestamps.push(l2.timestamp_str());
                                pixbufs.push(rgba_to_pixbuf(&img));
                                decoded.push((l2, vel));
                            }
                        }
                    }
                    if pixbufs.is_empty() {
                        on_done(Err("no animation frames decoded".to_string()));
                        return;
                    }
                    {
                        let mut st = state.borrow_mut();
                        st.anim_pixbufs = pixbufs;
                        st.anim_timestamps = timestamps;
                        st.anim_l2_frames = decoded;
                        st.anim_l3_frames.clear();
                        st.anim_index = 0;
                        st.current_pixbuf = st.anim_pixbufs.first().cloned();
                        st.timestamp_str = st.anim_timestamps.first().cloned();
                    }
                    drawing_area.queue_draw();
                    let n = state.borrow().anim_pixbufs.len();
                    on_done(Ok(n));
                }
                Err(e) => {
                    stop_progress.set(true);
                    status.set_text(&format!("Anim error: {e}"));
                    on_done(Err(e.to_string()));
                }
            },
        );
    } else {
        let progress_c = Arc::clone(&progress);
        runtime::spawn(
            async move {
                if let Ok(mut g) = progress_c.lock() {
                    *g = Some("Reading Level 3 frame inventory...".to_string());
                }
                let client = meso_data::http::wx_client();
                let dl = RadarDownloader::new(client);
                let filenames = dl
                    .level3_filenames_for_animation(&site_id, &product, frame_count)
                    .await?;
                let mut frames = Vec::new();
                let total = filenames.len();
                for (i, fname) in filenames.iter().enumerate() {
                    if let Ok(mut g) = progress_c.lock() {
                        *g = Some(format!("Fetching frame {}/{}", i + 1, total));
                    }
                    if let Some(url) = RadarDownloader::level3_file_url(&site_id, &product, fname) {
                        frames.push(dl.fetch_bytes(&url).await?);
                    }
                }
                Ok::<_, anyhow::Error>(frames)
            },
            move |result| match result {
                Ok(raw_frames) => {
                    stop_progress.set(true);
                    status.set_text(&format!(
                        "Constructing animation from {} frames...",
                        raw_frames.len()
                    ));
                    let mut pixbufs = Vec::new();
                    let mut timestamps = Vec::new();
                    let mut decoded = Vec::new();
                    for raw in raw_frames {
                        let st = state.borrow();
                        let is_vel = st.product.is_velocity();
                        if let Ok(l3) = level3::decode(&raw) {
                            let palette = st.palette_registry.for_product(l3.product_code);
                            if let Ok(img) = cairo_render::render_level3(
                                &l3,
                                palette,
                                &st.viewport,
                                &st.overlays,
                                is_vel,
                                Some(st.map_data.as_ref()),
                            ) {
                                timestamps.push(l3.timestamp_str());
                                pixbufs.push(rgba_to_pixbuf(&img));
                                decoded.push(l3);
                            }
                        }
                    }
                    if pixbufs.is_empty() {
                        on_done(Err("no animation frames decoded".to_string()));
                        return;
                    }
                    {
                        let mut st = state.borrow_mut();
                        st.anim_pixbufs = pixbufs;
                        st.anim_timestamps = timestamps;
                        st.anim_l3_frames = decoded;
                        st.anim_l2_frames.clear();
                        st.anim_index = 0;
                        st.current_pixbuf = st.anim_pixbufs.first().cloned();
                        st.timestamp_str = st.anim_timestamps.first().cloned();
                    }
                    drawing_area.queue_draw();
                    let n = state.borrow().anim_pixbufs.len();
                    on_done(Ok(n));
                }
                Err(e) => {
                    stop_progress.set(true);
                    status.set_text(&format!("Anim error: {e}"));
                    on_done(Err(e.to_string()));
                }
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn start_animation(
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    status: Label,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    frame_count: usize,
    anim_btn: Button,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
) {
    running.set(true);
    anim_btn.set_sensitive(false);
    let product = state.borrow().product;
    let site_id = state.borrow().site_id.clone();

    // Progress slot: async task writes "Fetching frame N/M", GTK poller reads it
    let progress: runtime::ProgressSlot = Arc::new(Mutex::new(None));
    let stop_progress = runtime::progress_poller(Arc::clone(&progress), status.clone());

    if product.is_level2() {
        let velocity = product == RadarProduct::L2Velocity;
        let progress_c = Arc::clone(&progress);
        runtime::spawn(
            async move {
                let client = meso_data::http::wx_client();
                let dl = RadarDownloader::new(client);
                let l2_product = if velocity {
                    RadarProduct::L2Velocity
                } else {
                    RadarProduct::L2Reflectivity
                };
                let filenames = dl
                    .level2_filenames_for_animation(&site_id, frame_count)
                    .await?;
                let total = filenames.len();
                let base = RadarDownloader::level2_dir_url(&site_id);
                let mut frames = Vec::new();
                for (i, fname) in filenames.iter().enumerate() {
                    if let Ok(mut g) = progress_c.lock() {
                        *g = Some(format!("Fetching/decompressing frame {}/{}", i + 1, total));
                    }
                    let url = format!("{base}{fname}");
                    let decompressed = dl
                        .fetch_level2_decompressed(&site_id, &l2_product, &url)
                        .await?;
                    frames.push(decompressed);
                }
                Ok::<_, anyhow::Error>((frames, velocity))
            },
            move |result| {
                stop_progress.set(true);
                match result {
                    Ok((decomp_frames, vel)) => {
                        let total = decomp_frames.len();
                        status.set_text(&format!("Rendering 0/{total}..."));
                        render_l2_frames_idle(
                            decomp_frames,
                            vel,
                            total,
                            state,
                            drawing_area,
                            status,
                            running,
                            timer,
                            anim_btn,
                            timeline,
                            slider_updating,
                        );
                    }
                    Err(e) => {
                        anim_btn.set_sensitive(true);
                        status.set_text(&format!("Anim error: {e}"));
                        running.set(false);
                    }
                }
            },
        );
    } else {
        let progress_c = Arc::clone(&progress);
        runtime::spawn(
            async move {
                let client = meso_data::http::wx_client();
                let dl = RadarDownloader::new(client);
                let filenames = dl
                    .level3_filenames_for_animation(&site_id, &product, frame_count)
                    .await?;
                let total = filenames.len();
                let mut frames = Vec::new();
                for (i, fname) in filenames.iter().enumerate() {
                    if let Ok(mut g) = progress_c.lock() {
                        *g = Some(format!("Fetching frame {}/{}", i + 1, total));
                    }
                    if let Some(url) = RadarDownloader::level3_file_url(&site_id, &product, fname) {
                        let bytes = dl.fetch_bytes(&url).await?;
                        frames.push(bytes);
                    }
                }
                Ok::<_, anyhow::Error>(frames)
            },
            move |result| {
                stop_progress.set(true);
                match result {
                    Ok(raw_frames) => {
                        let total = raw_frames.len();
                        status.set_text(&format!("Rendering 0/{total}..."));
                        render_l3_frames_idle(
                            raw_frames,
                            total,
                            state,
                            drawing_area,
                            status,
                            running,
                            timer,
                            anim_btn,
                            timeline,
                            slider_updating,
                        );
                    }
                    Err(e) => {
                        anim_btn.set_sensitive(true);
                        status.set_text(&format!("Anim error: {e}"));
                        running.set(false);
                    }
                }
            },
        );
    }
}

/// Format the time span between the first and last frame timestamps.
/// Returns a string like " | 1h 25m" or " | 45m", or empty string if not parseable.
fn time_span_str(timestamps: &[String]) -> String {
    if timestamps.len() < 2 {
        return String::new();
    }
    let first = timestamps.first().unwrap();
    let last = timestamps.last().unwrap();
    // Timestamps are like "2026-05-23 01:20 EDT" — strip the timezone suffix and
    // parse with NaiveDateTime (only date+time parts matter for span calculation).
    let parse_ts = |s: &str| -> Option<NaiveDateTime> {
        // Take only the first two whitespace-delimited tokens: date and time.
        let mut parts = s.split_whitespace();
        let date = parts.next()?;
        let time = parts.next()?;
        NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M").ok()
    };
    if let (Some(t0), Some(t1)) = (parse_ts(first), parse_ts(last)) {
        let secs = (t1 - t0).num_seconds().abs();
        if secs == 0 {
            return String::new();
        }
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if h > 0 {
            format!(" | {h}h {m}m")
        } else {
            format!(" | {m}m")
        }
    } else {
        String::new()
    }
}

/// Process L2 animation frames one at a time on the GTK main loop via idle callbacks.
/// Frames are already decompressed (BZ2 expansion done in async task).
/// Writes each frame into state immediately so animation starts on the first decoded frame.
#[allow(clippy::too_many_arguments)]
fn render_l2_frames_idle(
    decomp_frames: Vec<Vec<u8>>,
    vel: bool,
    total: usize,
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    status: Label,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    anim_btn: Button,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
) {
    // Clear any stale animation state so frames append cleanly.
    {
        let mut st = state.borrow_mut();
        st.anim_pixbufs.clear();
        st.anim_timestamps.clear();
        st.anim_l2_frames.clear();
        st.anim_l3_frames.clear();
        st.anim_index = 0;
    }
    let frame_iter: Rc<RefCell<std::vec::IntoIter<Vec<u8>>>> =
        Rc::new(RefCell::new(decomp_frames.into_iter()));
    let idx = Rc::new(Cell::new(0usize));
    let timer_started = Rc::new(Cell::new(false));

    glib::idle_add_local(move || {
        let decompressed = { frame_iter.borrow_mut().next() };
        if let Some(decompressed) = decompressed {
            let i = idx.get();
            idx.set(i + 1);
            status.set_text(&format!("Rendering frame {}/{total}...", i + 1));
            let tilt_idx = state.borrow().l2_tilt_idx;
            // Render while holding an immutable borrow, then drop before mutating state.
            let rendered: Option<(String, Pixbuf, Level2Data)> = {
                let st = state.borrow();
                level2::decode(&decompressed, vel, tilt_idx)
                    .ok()
                    .and_then(|l2| {
                        let code: u16 = if vel { 99 } else { 94 };
                        let palette = st.palette_registry.for_product(code);
                        cairo_render::render_level2(
                            &l2,
                            palette,
                            &st.viewport,
                            &st.overlays,
                            vel,
                            Some(st.map_data.as_ref()),
                        )
                        .ok()
                        .map(|img| (l2.timestamp_str(), rgba_to_pixbuf(&img), l2))
                    })
            };
            if let Some((ts, pixbuf, l2)) = rendered {
                let is_first = {
                    let mut st = state.borrow_mut();
                    st.anim_timestamps.push(ts);
                    st.anim_pixbufs.push(pixbuf);
                    st.anim_l2_frames.push((l2, vel));
                    let first = st.anim_pixbufs.len() == 1;
                    if first {
                        st.current_pixbuf = st.anim_pixbufs.first().cloned();
                        st.timestamp_str = st.anim_timestamps.first().cloned();
                    }
                    first
                };
                drawing_area.queue_draw();
                if is_first && !timer_started.get() {
                    timer_started.set(true);
                    start_timer(
                        Rc::clone(&state),
                        drawing_area.clone(),
                        Rc::clone(&running),
                        Rc::clone(&timer),
                        timeline.clone(),
                        Rc::clone(&slider_updating),
                    );
                }
            }
            glib::ControlFlow::Continue
        } else {
            // All frames rendered; finalize the UI.
            let n = state.borrow().anim_pixbufs.len();
            anim_btn.set_sensitive(true);
            if n == 0 {
                status.set_text("Animation: no frames decoded");
                running.set(false);
            } else {
                let ts = state.borrow().anim_timestamps.clone();
                let span = time_span_str(&ts);
                let cur_idx = state.borrow().anim_index;
                // Update timeline range now that the final frame count is known.
                slider_updating.set(true);
                timeline.set_range(0.0, (n - 1) as f64);
                timeline.set_value(cur_idx as f64);
                timeline.set_sensitive(true);
                slider_updating.set(false);
                status.set_text(&format!("Animating {n} frames{span}"));
                anim_btn.set_label("⏸ Pause");
            }
            glib::ControlFlow::Break
        }
    });
}

/// Process L3 animation frames one at a time on the GTK main loop via idle callbacks.
/// Writes each frame into state immediately so animation starts on the first decoded frame.
#[allow(clippy::too_many_arguments)]
fn render_l3_frames_idle(
    raw_frames: Vec<Vec<u8>>,
    total: usize,
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    status: Label,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    anim_btn: Button,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
) {
    // Clear any stale animation state so frames append cleanly.
    {
        let mut st = state.borrow_mut();
        st.anim_pixbufs.clear();
        st.anim_timestamps.clear();
        st.anim_l3_frames.clear();
        st.anim_l2_frames.clear();
        st.anim_index = 0;
    }
    let raw_iter: Rc<RefCell<std::vec::IntoIter<Vec<u8>>>> =
        Rc::new(RefCell::new(raw_frames.into_iter()));
    let idx = Rc::new(Cell::new(0usize));
    let timer_started = Rc::new(Cell::new(false));

    glib::idle_add_local(move || {
        let raw = { raw_iter.borrow_mut().next() };
        if let Some(raw) = raw {
            let i = idx.get();
            idx.set(i + 1);
            status.set_text(&format!("Rendering frame {}/{total}...", i + 1));
            // Render while holding an immutable borrow, then drop before mutating state.
            let rendered: Option<(String, Pixbuf, Level3Data)> = {
                let st = state.borrow();
                let is_vel = st.product.is_velocity();
                level3::decode(&raw).ok().and_then(|l3| {
                    let palette = st.palette_registry.for_product(l3.product_code);
                    cairo_render::render_level3(
                        &l3,
                        palette,
                        &st.viewport,
                        &st.overlays,
                        is_vel,
                        Some(st.map_data.as_ref()),
                    )
                    .ok()
                    .map(|img| (l3.timestamp_str(), rgba_to_pixbuf(&img), l3))
                })
            };
            if let Some((ts, pixbuf, l3)) = rendered {
                let is_first = {
                    let mut st = state.borrow_mut();
                    st.anim_timestamps.push(ts);
                    st.anim_pixbufs.push(pixbuf);
                    st.anim_l3_frames.push(l3);
                    let first = st.anim_pixbufs.len() == 1;
                    if first {
                        st.current_pixbuf = st.anim_pixbufs.first().cloned();
                        st.timestamp_str = st.anim_timestamps.first().cloned();
                    }
                    first
                };
                drawing_area.queue_draw();
                if is_first && !timer_started.get() {
                    timer_started.set(true);
                    start_timer(
                        Rc::clone(&state),
                        drawing_area.clone(),
                        Rc::clone(&running),
                        Rc::clone(&timer),
                        timeline.clone(),
                        Rc::clone(&slider_updating),
                    );
                }
            }
            glib::ControlFlow::Continue
        } else {
            // All frames rendered; finalize the UI.
            let n = state.borrow().anim_pixbufs.len();
            anim_btn.set_sensitive(true);
            if n == 0 {
                status.set_text("Animation: no frames decoded");
                running.set(false);
            } else {
                let ts = state.borrow().anim_timestamps.clone();
                let span = time_span_str(&ts);
                let cur_idx = state.borrow().anim_index;
                // Update timeline range now that the final frame count is known.
                slider_updating.set(true);
                timeline.set_range(0.0, (n - 1) as f64);
                timeline.set_value(cur_idx as f64);
                timeline.set_sensitive(true);
                slider_updating.set(false);
                status.set_text(&format!("Animating {n} frames{span}"));
                anim_btn.set_label("⏸ Pause");
            }
            glib::ControlFlow::Break
        }
    });
}

fn start_timer(
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
) {
    let id = glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if !running.get() {
            return glib::ControlFlow::Break;
        }
        let mut st = state.borrow_mut();
        if st.anim_pixbufs.is_empty() {
            return glib::ControlFlow::Break;
        }
        st.anim_index = (st.anim_index + 1) % st.anim_pixbufs.len();
        st.current_pixbuf = Some(st.anim_pixbufs[st.anim_index].clone());
        st.timestamp_str = st.anim_timestamps.get(st.anim_index).cloned();
        let idx = st.anim_index;
        drop(st);
        // Update timeline position without triggering the value_changed scrub handler
        slider_updating.set(true);
        timeline.set_value(idx as f64);
        slider_updating.set(false);
        drawing_area.queue_draw();
        glib::ControlFlow::Continue
    });
    *timer.borrow_mut() = Some(id);
}

/// Re-render all animation frames from decoded data at the current viewport.
/// Used after a zoom/pan debounce, or when the palette changes during animation.
/// Processes one frame per GTK idle slot to keep the UI responsive.
fn re_render_all_anim_frames_idle(state: Rc<RefCell<RadarPaneState>>, drawing_area: DrawingArea) {
    let total = {
        let st = state.borrow();
        st.anim_l2_frames.len().max(st.anim_l3_frames.len())
    };
    if total == 0 {
        return;
    }

    let idx = Rc::new(Cell::new(0usize));
    glib::idle_add_local(move || {
        let i = idx.get();
        if i >= total {
            drawing_area.queue_draw();
            return glib::ControlFlow::Break;
        }
        idx.set(i + 1);
        let mut st = state.borrow_mut();
        let map = Some(st.map_data.as_ref());
        if let Some((l2, vel)) = st.anim_l2_frames.get(i) {
            let code: u16 = if *vel { 99 } else { 94 };
            let palette = st.palette_registry.for_product(code);
            if let Ok(img) =
                cairo_render::render_level2(l2, palette, &st.viewport, &st.overlays, *vel, map)
            {
                let pb = rgba_to_pixbuf(&img);
                if let Some(slot) = st.anim_pixbufs.get_mut(i) {
                    *slot = pb.clone();
                }
                if i == st.anim_index {
                    st.current_pixbuf = Some(pb);
                }
            }
        } else if let Some(l3) = st.anim_l3_frames.get(i) {
            let is_vel = st.product.is_velocity();
            let palette = st.palette_registry.for_product(l3.product_code);
            if let Ok(img) =
                cairo_render::render_level3(l3, palette, &st.viewport, &st.overlays, is_vel, map)
            {
                let pb = rgba_to_pixbuf(&img);
                if let Some(slot) = st.anim_pixbufs.get_mut(i) {
                    *slot = pb.clone();
                }
                if i == st.anim_index {
                    st.current_pixbuf = Some(pb);
                }
            }
        }
        // Queue draw each frame so animation looks live while re-rendering
        drop(st);
        drawing_area.queue_draw();
        glib::ControlFlow::Continue
    });
}

// ── Data loading ──────────────────────────────────────────────────────────────

fn trigger_load(
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    status: Label,
    btns: Vec<Button>,
    tilt_combo: DropDown,
) {
    let product = state.borrow().product;
    match product {
        RadarProduct::L2Reflectivity => {
            load_level2(state, drawing_area, status, false, btns, tilt_combo)
        }
        RadarProduct::L2Velocity => {
            load_level2(state, drawing_area, status, true, btns, tilt_combo.clone())
        }
        _ => {
            tilt_combo.set_visible(false);
            load_level3(state, drawing_area, status, btns)
        }
    }
}

fn load_level3(
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    status: Label,
    btns: Vec<Button>,
) {
    let site_id = state.borrow().site_id.clone();
    let product = state.borrow().product;
    for b in &btns {
        b.set_sensitive(false);
    }
    status.set_text(&format!("Fetching {} {}...", site_id, product.label()));

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            let dl = RadarDownloader::new(client);
            dl.fetch_level3(&site_id, &product).await
        },
        move |result| {
            for b in &btns {
                b.set_sensitive(true);
            }
            match result {
                Ok(raw) => match level3::decode(&raw) {
                    Ok(l3) => {
                        let is_vel = state.borrow().product.is_velocity();
                        let (palette_kind, img) = {
                            let st = state.borrow();
                            let palette = st.palette_registry.for_product(l3.product_code);
                            let img = cairo_render::render_level3(
                                &l3,
                                palette,
                                &st.viewport,
                                &st.overlays,
                                is_vel,
                                Some(st.map_data.as_ref()),
                            );
                            (l3.product_code, img)
                        };
                        let _ = palette_kind;
                        state.borrow_mut().cached_l3 = Some(l3.clone());
                        match img {
                            Ok(img) => {
                                let mut st = state.borrow_mut();
                                st.timestamp_str = Some(l3.timestamp_str());
                                st.current_pixbuf = Some(rgba_to_pixbuf(&img));
                                let desc = st.product.description_line();
                                let status_text = if desc.is_empty() {
                                    "Ready".to_string()
                                } else {
                                    format!("Ready — {desc}")
                                };
                                status.set_text(&status_text);
                                drawing_area.queue_draw();
                            }
                            Err(e) => status.set_text(&format!("Render error: {e}")),
                        }
                    }
                    Err(e) => status.set_text(&format!("Decode error: {e}")),
                },
                Err(e) => status.set_text(&format!("Download error: {e}")),
            }
        },
    );
}

fn load_level2(
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    status: Label,
    velocity: bool,
    btns: Vec<Button>,
    tilt_combo: DropDown,
) {
    let site_id = state.borrow().site_id.clone();
    let tilt_idx = state.borrow().l2_tilt_idx;
    let label = if velocity {
        "L2 Velocity"
    } else {
        "L2 Reflectivity"
    };
    for b in &btns {
        b.set_sensitive(false);
    }
    status.set_text(&format!("Fetching {} {}...", site_id, label));

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            let dl = RadarDownloader::new(client);
            let prod = if velocity {
                RadarProduct::L2Velocity
            } else {
                RadarProduct::L2Reflectivity
            };
            dl.fetch_level2_partial(&site_id, &prod, None).await
        },
        move |result| {
            for b in &btns {
                b.set_sensitive(true);
            }
            match result {
                Ok(compressed) => match level2::decompress_level2(&compressed) {
                    Ok(raw) => {
                        // Populate tilt list
                        let tilts = level2::list_tilts(&raw, velocity).unwrap_or_default();
                        let safe_tilt = tilt_idx.min(tilts.len().saturating_sub(1));
                        match level2::decode(&raw, velocity, safe_tilt) {
                            Ok(l2) => {
                                let img = {
                                    let st = state.borrow();
                                    let code: u16 = if velocity { 99 } else { 94 };
                                    let palette = st.palette_registry.for_product(code);
                                    cairo_render::render_level2(
                                        &l2,
                                        palette,
                                        &st.viewport,
                                        &st.overlays,
                                        velocity,
                                        Some(st.map_data.as_ref()),
                                    )
                                };
                                {
                                    let mut st = state.borrow_mut();
                                    st.cached_l2 = Some((l2.clone(), velocity));
                                    st.cached_l2_bytes = Some(raw);
                                    st.l2_tilt_idx = safe_tilt;
                                    st.l2_tilts = tilts.clone();
                                }
                                // Update tilt selector (outside state borrow)
                                {
                                    let labels: Vec<String> = tilts
                                        .iter()
                                        .map(|t| format!("{:.1}°", t.angle_deg))
                                        .collect();
                                    let label_refs: Vec<&str> =
                                        labels.iter().map(String::as_str).collect();
                                    if let Some(strings) = tilt_combo
                                        .model()
                                        .and_then(|m| m.downcast::<gtk4::StringList>().ok())
                                    {
                                        strings.splice(0, strings.n_items(), &label_refs);
                                    }
                                    tilt_combo.set_selected(safe_tilt as u32);
                                }
                                tilt_combo.set_visible(true);
                                match img {
                                    Ok(img) => {
                                        let mut st = state.borrow_mut();
                                        st.timestamp_str = Some(l2.timestamp_str());
                                        st.current_pixbuf = Some(rgba_to_pixbuf(&img));
                                        status.set_text("Ready");
                                        drawing_area.queue_draw();
                                    }
                                    Err(e) => status.set_text(&format!("Render error: {e}")),
                                }
                            }
                            Err(e) => status.set_text(&format!("L2 decode error: {e}")),
                        }
                    }
                    Err(e) => status.set_text(&format!("Decompress error: {e}")),
                },
                Err(e) => status.set_text(&format!("L2 download error: {e}")),
            }
        },
    );
}

fn refresh_warnings(
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    cfg: Rc<RefCell<Config>>,
) {
    let site_id = state.borrow().site_id.clone();
    let site_ll = meso_data::geo::sites::site_latlon(&site_id).unwrap_or(LatLon {
        lat: 35.0,
        lon: -80.0,
    });
    let state_abbrev = state_from_lat_lon(site_ll.lat, site_ll.lon);
    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            fetch_active_alerts_by_state(&client, &state_abbrev).await
        },
        move |result| {
            if let Ok(warnings) = result {
                let show = cfg.borrow().radar_show_warnings;
                let layer = build_warnings_layer(&warnings);
                let has_anim = {
                    let mut st = state.borrow_mut();
                    st.warnings = warnings;
                    st.overlays.layers.retain(|l| l.name != "warnings");
                    let mut l = layer;
                    l.visible = show;
                    st.overlays.layers.push(l);
                    !st.anim_l2_frames.is_empty() || !st.anim_l3_frames.is_empty()
                };
                if has_anim {
                    re_render_all_anim_frames_idle(Rc::clone(&state), drawing_area.clone());
                } else {
                    let _ = state.borrow_mut().render_from_cache();
                    drawing_area.queue_draw();
                }
            }
        },
    );
}

fn refresh_storm_tracks(
    state: Rc<RefCell<RadarPaneState>>,
    drawing_area: DrawingArea,
    cfg: Rc<RefCell<Config>>,
) {
    let site_id = state.borrow().site_id.clone();
    let site_ll = meso_data::geo::sites::site_latlon(&site_id).unwrap_or(LatLon {
        lat: 35.0,
        lon: -80.0,
    });
    let (site_lat, site_lon) = (site_ll.lat, site_ll.lon);
    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            fetch_storm_tracks(&client, &site_id, site_lat, site_lon).await
        },
        move |result| {
            let show = cfg.borrow().radar_show_storm_tracks;
            let layer = match result {
                Ok(cells) => {
                    let mut l = build_storm_tracks_layer(&cells);
                    l.visible = show;
                    l
                }
                Err(e) => {
                    tracing::warn!("Storm tracks fetch error: {e}");
                    let mut l = OverlayLayer::new("storm_tracks");
                    l.visible = show;
                    l
                }
            };
            let has_anim = {
                let mut st = state.borrow_mut();
                st.overlays.layers.retain(|l| l.name != "storm_tracks");
                st.overlays.layers.push(layer);
                !st.anim_l2_frames.is_empty() || !st.anim_l3_frames.is_empty()
            };
            if has_anim {
                re_render_all_anim_frames_idle(Rc::clone(&state), drawing_area.clone());
            } else {
                let _ = state.borrow_mut().render_from_cache();
                drawing_area.queue_draw();
            }
        },
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn rgba_to_pixbuf(img: &meso_render::frame::RenderedImage) -> Pixbuf {
    Pixbuf::from_bytes(
        &glib::Bytes::from(&img.data),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        img.width as i32,
        img.height as i32,
        (img.width * 4) as i32,
    )
}

// ── Gate inspect helpers ──────────────────────────────────────────────────────

/// Compute the bearing (degrees, clockwise from North) from (lat1,lon1) to (lat2,lon2).
fn bearing_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let lat1r = lat1.to_radians();
    let lat2r = lat2.to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let y = dlon.sin() * lat2r.cos();
    let x = lat1r.cos() * lat2r.sin() - lat1r.sin() * lat2r.cos() * dlon.cos();
    let bearing = y.atan2(x).to_degrees();
    (bearing + 360.0) % 360.0
}

/// Find the nearest radial index in the L2 azimuths array.
fn nearest_radial(azimuths: &[f32], az: f64) -> usize {
    let az = az as f32;
    let mut best_idx = 0usize;
    let mut best_diff = 360.0f32;
    for (i, &a) in azimuths.iter().enumerate() {
        let mut diff = (a - az).abs();
        if diff > 180.0 {
            diff = 360.0 - diff;
        }
        if diff < best_diff {
            best_diff = diff;
            best_idx = i;
        }
    }
    best_idx
}

/// Attempt to look up a gate value from the cached L2 data.
/// Returns `Some(("Ref", dbz_value))` or `Some(("Vel", ms_value))` or None.
fn lookup_l2_gate(st: &RadarPaneState, range_km: f64, az: f64) -> Option<(String, f64)> {
    use meso_data::radar::level2::NUM_RANGE_BINS;

    // Try current cached L2 first; fall back to first animation frame
    let (data, is_vel) = if let Some(ref l2) = st.cached_l2 {
        (&l2.0, l2.1)
    } else if let Some(first) = st.anim_l2_frames.first() {
        (&first.0, first.1)
    } else {
        return None;
    };

    let bin_size = data.bin_size_km as f64;
    let bin_idx = (range_km / bin_size) as usize;
    if bin_idx >= NUM_RANGE_BINS {
        return None;
    }

    let radial_idx = nearest_radial(&data.azimuths, az);
    let raw = data.bins[radial_idx * NUM_RANGE_BINS + bin_idx];

    if raw < 2 {
        return None; // below threshold or range-folded
    }

    if is_vel {
        // L2 velocity encoding: dbz = raw/2 - 63.5 (m/s, then convert to knots display)
        let ms = raw as f64 / 2.0 - 63.5;
        let kt = ms * 1.944;
        Some(("Vel".to_string(), kt))
    } else {
        // L2 reflectivity: dBZ = raw/2 - 32
        let dbz = raw as f64 / 2.0 - 32.0;
        Some(("Ref".to_string(), dbz))
    }
}

/// Attempt to look up a gate value from cached L3 data.
fn lookup_l3_gate(st: &RadarPaneState, range_km: f64, az: f64) -> Option<(String, f64)> {
    // Try current cached L3 first; fall back to first animation frame
    let data = if let Some(ref l3) = st.cached_l3 {
        l3
    } else {
        st.anim_l3_frames.first()?
    };

    let raw = if data.is_raster {
        let cell_km = data.bin_size_km as f64;
        let x_km = range_km * az.to_radians().sin();
        let y_km = range_km * az.to_radians().cos();
        let half_rows = data.num_radials as f64 / 2.0;
        let half_cols = data.num_range_bins as f64 / 2.0;
        let row = (half_rows - y_km / cell_km).floor() as isize;
        let col = (x_km / cell_km + half_cols).floor() as isize;
        if row < 0
            || col < 0
            || row as usize >= data.num_radials
            || col as usize >= data.num_range_bins
        {
            return None;
        }
        data.bins
            .get(row as usize * data.num_range_bins + col as usize)
            .copied()?
    } else {
        let bin_size = data.bin_size_km as f64;
        let bin_idx = (range_km / bin_size) as usize;
        if bin_idx >= data.bins.len() / data.num_radials.max(1) {
            return None;
        }

        let radial_idx = nearest_radial(&data.azimuths, az);
        let num_bins = data.bins.len() / data.num_radials.max(1);
        data.bins.get(radial_idx * num_bins + bin_idx).copied()?
    };

    if raw < 2 {
        return None;
    }

    // L3 products: use product-appropriate label
    let label = if st.product.is_velocity() {
        "Vel".to_string()
    } else {
        "Ref".to_string()
    };
    // Generic: raw value as dBZ equivalent (most products use same scale as L2)
    let dbz = raw as f64 / 2.0 - 32.0;
    Some((label, dbz))
}

fn build_inspect_message(st: &RadarPaneState, clicked_ll: &LatLon) -> String {
    let site = &st.viewport.site_origin;
    let range_km = site.distance_km(clicked_ll);
    let az = bearing_deg(site.lat, site.lon, clicked_ll.lat, clicked_ll.lon);
    let gate_info = lookup_l2_gate(st, range_km, az).or_else(|| lookup_l3_gate(st, range_km, az));
    let ns = if clicked_ll.lat >= 0.0 { 'N' } else { 'S' };
    let ew = if clicked_ll.lon >= 0.0 { 'E' } else { 'W' };
    if let Some((label, value)) = gate_info {
        format!(
            "{:.3}°{ns} {:.3}°{ew}  Az:{:.0}°  Rng:{:.1}km  {}: {:.1}",
            clicked_ll.lat.abs(),
            clicked_ll.lon.abs(),
            az,
            range_km,
            label,
            value
        )
    } else {
        format!(
            "{:.3}°{ns} {:.3}°{ew}  Az:{:.0}°  Rng:{:.1}km  (no data)",
            clicked_ll.lat.abs(),
            clicked_ll.lon.abs(),
            az,
            range_km
        )
    }
}

fn ensure_active_track_index(cfg: &mut Config) -> usize {
    if cfg.radar_active_track_id.is_empty() {
        cfg.radar_active_track_id = "default".to_string();
    }
    if let Some(idx) = cfg
        .radar_tracks
        .iter()
        .position(|t| t.id == cfg.radar_active_track_id)
    {
        return idx;
    }
    let id = cfg.radar_active_track_id.clone();
    let name = if id == "default" {
        "Default Track".to_string()
    } else {
        id.clone()
    };
    cfg.radar_tracks.push(RadarTrack {
        id,
        name,
        points: Vec::new(),
    });
    cfg.radar_tracks.len() - 1
}

fn append_track_point(cfg: &mut Config, point: RadarTrackPoint) -> usize {
    let idx = ensure_active_track_index(cfg);
    cfg.radar_tracks[idx].points.push(point);
    idx
}

fn clear_active_track(cfg: &mut Config) -> bool {
    let idx = ensure_active_track_index(cfg);
    if cfg.radar_tracks[idx].points.is_empty() {
        return false;
    }
    cfg.radar_tracks[idx].points.clear();
    true
}

fn remove_nearest_track_point(cfg: &mut Config, clicked: &LatLon, max_distance_km: f64) -> bool {
    let idx = ensure_active_track_index(cfg);
    let points = &mut cfg.radar_tracks[idx].points;
    if points.is_empty() {
        return false;
    }
    let mut best_idx = 0usize;
    let mut best_dist = f64::MAX;
    for (i, p) in points.iter().enumerate() {
        let d = clicked.distance_km(&LatLon {
            lat: p.lat,
            lon: p.lon,
        });
        if d < best_dist {
            best_dist = d;
            best_idx = i;
        }
    }
    if best_dist > max_distance_km {
        return false;
    }
    points.remove(best_idx);
    true
}

fn show_location_editor_dialog(
    default_name: &str,
    lat: f64,
    lon: f64,
    shared_config: Rc<RefCell<Config>>,
    status: gtk4::Label,
    left_da: gtk4::DrawingArea,
    right_da: gtk4::DrawingArea,
) {
    use gtk4::{Box as GBox, Button, Entry, Label, Orientation};

    let win = gtk4::Window::new();
    win.set_title(Some("Add Location"));
    win.set_modal(true);
    win.set_default_size(300, 220);

    let content = GBox::new(Orientation::Vertical, 6);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Name
    let name_label = Label::new(Some("Name:"));
    name_label.set_halign(gtk4::Align::Start);
    let name_entry = Entry::new();
    name_entry.set_text(default_name);
    content.append(&name_label);
    content.append(&name_entry);

    // Latitude
    let lat_label = Label::new(Some("Latitude:"));
    lat_label.set_halign(gtk4::Align::Start);
    let lat_entry = Entry::new();
    lat_entry.set_text(&format!("{:.4}", lat));
    content.append(&lat_label);
    content.append(&lat_entry);

    // Longitude
    let lon_label = Label::new(Some("Longitude:"));
    lon_label.set_halign(gtk4::Align::Start);
    let lon_entry = Entry::new();
    lon_entry.set_text(&format!("{:.4}", lon));
    content.append(&lon_label);
    content.append(&lon_entry);

    let btn_row = GBox::new(Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_top(8);
    let cancel_btn = Button::with_label("Cancel");
    let save_btn = Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    content.append(&btn_row);

    win.set_child(Some(&content));

    let win_cancel = win.clone();
    cancel_btn.connect_clicked(move |_| win_cancel.close());

    let win_save = win.clone();
    let name_entry_c = name_entry.clone();
    let lat_entry_c = lat_entry.clone();
    let lon_entry_c = lon_entry.clone();
    save_btn.connect_clicked(move |_| {
        let new_name = name_entry_c.text().trim().to_string();
        let new_lat_s = lat_entry_c.text();
        let new_lon_s = lon_entry_c.text();

        if new_name.is_empty() {
            win_save.close();
            return;
        }

        let new_lat = match new_lat_s.trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => {
                win_save.close();
                return;
            }
        };

        let new_lon = match new_lon_s.trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => {
                win_save.close();
                return;
            }
        };

        if !(-90.0..=90.0).contains(&new_lat) || !(-180.0..=180.0).contains(&new_lon) {
            win_save.close();
            return;
        }

        {
            let mut cfg = shared_config.borrow_mut();
            cfg.locations.push(NamedLocation {
                name: new_name.clone(),
                lat: new_lat,
                lon: new_lon,
            });
        }

        status.set_text(&format!("Added location: {new_name}"));
        left_da.queue_draw();
        right_da.queue_draw();
        win_save.close();
    });

    win.present();
}

// ── Location marker drawing ───────────────────────────────────────────────────

/// Draw a simple dot marker for each named location.
///
/// Active location → cyan dot (r=5); inactive → yellow dot (r=5).
/// Name label drawn to the right of the dot in white with a dark outline.
/// Crude but fast lat/lon → US state code lookup via bounding boxes.
/// Falls back to "US" if outside all known boxes.
fn state_from_lat_lon(lat: f64, lon: f64) -> String {
    #[rustfmt::skip]
    let boxes: &[(&str, f64, f64, f64, f64)] = &[
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

fn draw_major_roads(
    cr: &gtk4::cairo::Context,
    w: i32,
    h: i32,
    viewport: &meso_render::viewport::Viewport,
    map_data: &MapData,
    cfg: &crate::config::Config,
) {
    if !cfg.radar_show_major_roads {
        return;
    }

    let width = w as f64;
    let height = h as f64;
    let margin = 24.0_f64;

    cr.set_source_rgba(0.96, 0.96, 0.96, 0.85);
    cr.set_line_width(0.9);

    for seg in &map_data.roads_major {
        let (x1, y1) = viewport.latlon_to_screen(&LatLon {
            lat: seg.lat1 as f64,
            lon: seg.lon1 as f64,
        });
        let (x2, y2) = viewport.latlon_to_screen(&LatLon {
            lat: seg.lat2 as f64,
            lon: seg.lon2 as f64,
        });

        let min_x = x1.min(x2);
        let max_x = x1.max(x2);
        let min_y = y1.min(y2);
        let max_y = y1.max(y2);
        if max_x < -margin || min_x > width + margin || max_y < -margin || min_y > height + margin {
            continue;
        }

        cr.move_to(x1, y1);
        cr.line_to(x2, y2);
    }
    let _ = cr.stroke();
}

fn draw_location_markers(
    cr: &gtk4::cairo::Context,
    w: i32,
    h: i32,
    viewport: &meso_render::viewport::Viewport,
    cfg: &crate::config::Config,
) {
    let active_name = &cfg.active_location;
    for loc in &cfg.locations {
        let ll = LatLon {
            lat: loc.lat,
            lon: loc.lon,
        };
        let (sx, sy) = viewport.latlon_to_screen(&ll);
        if sx < -8.0 || sy < -8.0 || sx > w as f64 + 8.0 || sy > h as f64 + 8.0 {
            continue;
        }

        let is_active = loc.name == *active_name;
        if is_active {
            cr.set_source_rgb(0.0, 1.0, 1.0); // cyan
        } else {
            cr.set_source_rgb(1.0, 1.0, 0.0); // yellow
        }

        // Filled dot
        cr.arc(sx, sy, 5.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();

        // Name label with dark outline
        cr.select_font_face(
            "Monospace",
            gtk4::cairo::FontSlant::Normal,
            gtk4::cairo::FontWeight::Bold,
        );
        cr.set_font_size(11.0);
        let lx = sx + 8.0;
        let ly = sy + 4.0;
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.8);
        for dx in [-1.0_f64, 0.0, 1.0] {
            for dy in [-1.0_f64, 0.0, 1.0] {
                if dx != 0.0 || dy != 0.0 {
                    cr.move_to(lx + dx, ly + dy);
                    let _ = cr.show_text(&loc.name);
                }
            }
        }
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(lx, ly);
        let _ = cr.show_text(&loc.name);
    }
}

fn marker_time_minutes(point: &RadarTrackPoint) -> Option<i64> {
    if let Some(ft) = point.frame_time.as_deref() {
        let mut parts = ft.split_whitespace();
        if let (Some(date), Some(time)) = (parts.next(), parts.next()) {
            if let Ok(dt) =
                NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M")
            {
                return Some(dt.and_utc().timestamp() / 60);
            }
        }
    }
    DateTime::parse_from_rfc3339(&point.created_at)
        .ok()
        .map(|dt| dt.timestamp() / 60)
}

fn latlon_to_xy_km(lat: f64, lon: f64, lat0: f64, lon0: f64) -> (f64, f64) {
    let r_km = 6371.0;
    let x = (lon - lon0).to_radians() * r_km * lat0.to_radians().cos();
    let y = (lat - lat0).to_radians() * r_km;
    (x, y)
}

fn xy_km_to_latlon(x: f64, y: f64, lat0: f64, lon0: f64) -> LatLon {
    let r_km = 6371.0;
    let lat = lat0 + (y / r_km).to_degrees();
    let cos_lat = lat0.to_radians().cos().abs().max(1e-6);
    let lon = lon0 + (x / (r_km * cos_lat)).to_degrees();
    LatLon { lat, lon }
}

fn projected_track_points(track: &RadarTrack, cfg: &Config) -> Vec<LatLon> {
    if track.points.len() < 2 {
        return Vec::new();
    }

    // Sort points chronologically so projection always moves forward in time
    // regardless of the order the user placed markers.
    let mut sorted: Vec<&RadarTrackPoint> = track.points.iter().collect();
    sorted.sort_by_key(|p| marker_time_minutes(p).unwrap_or_else(|| p.frame_index as i64 * 5));

    let last = sorted.last().unwrap();
    let lat0 = last.lat;
    let lon0 = last.lon;

    let mut seg_vel: Vec<(f64, f64, f64)> = Vec::new();
    for i in 1..sorted.len() {
        let a = sorted[i - 1];
        let b = sorted[i];
        let dt_minutes = match (marker_time_minutes(a), marker_time_minutes(b)) {
            (Some(t0), Some(t1)) if t1 > t0 => (t1 - t0) as f64,
            _ if b.frame_index > a.frame_index => (b.frame_index - a.frame_index) as f64 * 5.0,
            _ => 5.0,
        }
        .max(1.0);

        let (ax, ay) = latlon_to_xy_km(a.lat, a.lon, lat0, lon0);
        let (bx, by) = latlon_to_xy_km(b.lat, b.lon, lat0, lon0);
        seg_vel.push(((bx - ax) / dt_minutes, (by - ay) / dt_minutes, dt_minutes));
    }
    if seg_vel.is_empty() {
        return Vec::new();
    }

    let mut v_sum_x = 0.0;
    let mut v_sum_y = 0.0;
    let mut w_sum = 0.0;
    for (i, (vx, vy, _)) in seg_vel.iter().enumerate() {
        let w = (i + 1) as f64;
        v_sum_x += vx * w;
        v_sum_y += vy * w;
        w_sum += w;
    }
    let mut vx = if w_sum > 0.0 { v_sum_x / w_sum } else { 0.0 };
    let mut vy = if w_sum > 0.0 { v_sum_y / w_sum } else { 0.0 };

    let mut ax = 0.0;
    let mut ay = 0.0;
    if cfg.radar_vector_accel_bias && seg_vel.len() >= 2 {
        let (vx0, vy0, dt0) = seg_vel[seg_vel.len() - 2];
        let (vx1, vy1, dt1) = seg_vel[seg_vel.len() - 1];
        let dt = ((dt0 + dt1) / 2.0).max(1.0);
        ax = (vx1 - vx0) / dt;
        ay = (vy1 - vy0) / dt;
        vx = (vx + vx1) / 2.0;
        vy = (vy + vy1) / 2.0;
    }

    let mut projected = Vec::new();
    let lead = cfg.radar_vector_lead_minutes.max(15);
    let step = cfg.radar_vector_interval_minutes.max(5);
    let mut minute = step;
    while minute <= lead {
        let t = minute as f64;
        let x = vx * t + 0.5 * ax * t * t;
        let y = vy * t + 0.5 * ay * t * t;
        projected.push(xy_km_to_latlon(x, y, lat0, lon0));
        minute = minute.saturating_add(step);
    }
    projected
}

fn draw_custom_tracks(
    cr: &gtk4::cairo::Context,
    w: i32,
    h: i32,
    viewport: &meso_render::viewport::Viewport,
    cfg: &crate::config::Config,
) {
    for track in &cfg.radar_tracks {
        if track.points.is_empty() {
            continue;
        }

        let is_active = track.id == cfg.radar_active_track_id;

        // Sort markers chronologically so the drawn path always moves forward in time.
        let mut sorted_pts: Vec<&RadarTrackPoint> = track.points.iter().collect();
        sorted_pts
            .sort_by_key(|p| marker_time_minutes(p).unwrap_or_else(|| p.frame_index as i64 * 5));

        let mut screen_points: Vec<(f64, f64)> = Vec::new();
        for p in &sorted_pts {
            let (sx, sy) = viewport.latlon_to_screen(&LatLon {
                lat: p.lat,
                lon: p.lon,
            });
            if sx < -16.0 || sy < -16.0 || sx > w as f64 + 16.0 || sy > h as f64 + 16.0 {
                continue;
            }
            screen_points.push((sx, sy));
        }
        if screen_points.is_empty() {
            continue;
        }

        if cfg.radar_show_track_lines && screen_points.len() >= 2 {
            if is_active {
                cr.set_source_rgba(0.0, 0.9, 1.0, 0.9);
            } else {
                cr.set_source_rgba(0.8, 0.8, 0.8, 0.7);
            }
            cr.set_line_width(2.0);
            cr.move_to(screen_points[0].0, screen_points[0].1);
            for (sx, sy) in screen_points.iter().skip(1) {
                cr.line_to(*sx, *sy);
            }
            let _ = cr.stroke();
        }

        if cfg.radar_show_track_points {
            for (i, (sx, sy)) in screen_points.iter().enumerate() {
                if i + 1 == screen_points.len() {
                    cr.set_source_rgba(1.0, 0.2, 1.0, 0.95);
                    cr.arc(*sx, *sy, 4.5, 0.0, std::f64::consts::TAU);
                } else if is_active {
                    cr.set_source_rgba(0.0, 1.0, 1.0, 0.9);
                    cr.arc(*sx, *sy, 3.2, 0.0, std::f64::consts::TAU);
                } else {
                    cr.set_source_rgba(1.0, 1.0, 0.5, 0.85);
                    cr.arc(*sx, *sy, 2.8, 0.0, std::f64::consts::TAU);
                }
                let _ = cr.fill();
            }
        }

        if cfg.radar_show_track_vector {
            let projected = projected_track_points(track, cfg);
            if projected.is_empty() {
                continue;
            }
            let mut prev = LatLon {
                lat: sorted_pts.last().map(|p| p.lat).unwrap_or(0.0),
                lon: sorted_pts.last().map(|p| p.lon).unwrap_or(0.0),
            };
            let mut seg_index = 1usize;
            for ll in projected {
                let (x0, y0) = viewport.latlon_to_screen(&prev);
                let (x1, y1) = viewport.latlon_to_screen(&ll);
                if (x0 < -32.0 && x1 < -32.0)
                    || (x0 > w as f64 + 32.0 && x1 > w as f64 + 32.0)
                    || (y0 < -32.0 && y1 < -32.0)
                    || (y0 > h as f64 + 32.0 && y1 > h as f64 + 32.0)
                {
                    prev = ll;
                    seg_index += 1;
                    continue;
                }

                cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
                cr.set_line_width(1.6);
                cr.set_dash(&[4.0, 4.0], 0.0);
                cr.move_to(x0, y0);
                cr.line_to(x1, y1);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);

                cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                cr.select_font_face(
                    "Monospace",
                    gtk4::cairo::FontSlant::Normal,
                    gtk4::cairo::FontWeight::Bold,
                );
                cr.set_font_size(10.0);
                let minute = seg_index as u16 * cfg.radar_vector_interval_minutes.max(5);
                cr.move_to(x1 + 4.0, y1 - 4.0);
                let _ = cr.show_text(&format!("+{minute}m"));

                prev = ll;
                seg_index += 1;
            }
        }
    }
}
