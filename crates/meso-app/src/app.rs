/*
 * Main GTK4 application: window, tabs, and application lifecycle.
 *
 * Uses libadwaita for GNOME HIG compliance and dark/light theme support.
 * Main window layout:
 *   AdwApplicationWindow
 *     AdwHeaderBar (title)
 *     Box(Horizontal):
 *       Box(Vertical) [sidebar]:
 *         ListBox  (nav rows)
 *         (spacer)
 *         Separator
 *         Box(H) [📍 Locations | ⚙ Settings]
 *       Separator(Vertical)
 *       Stack (pane content, switched by sidebar selection)
 */

use gtk4::prelude::*;
use gtk4::{Box as GBox, Button, CssProvider, Label, ListBox, Orientation, Separator, Stack};
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow, HeaderBar};

use std::cell::RefCell;
use std::rc::Rc;

use crate::alerts_pane;
use crate::config::Config;
use crate::forecast_pane;
use crate::location_panel;
use crate::models_pane;
use crate::national_pane;
use crate::observations_pane;
use crate::radar_pane;
use crate::satellite_pane;
use crate::settings_panel;
use crate::soundings_pane;
use crate::spc_pane;
use crate::text_pane;

const APP_ID: &str = "org.meso.desktop";

pub struct WxApplication {
    app: Application,
}

impl WxApplication {
    pub fn new() -> Self {
        let app = Application::builder().application_id(APP_ID).build();

        let wx = WxApplication { app };
        wx.app.connect_startup(|_| {
            load_css();
            // Purge cache entries older than 24h at startup.
            // Weather data older than a day has no operational value.
            std::thread::spawn(|| {
                meso_data::cache::Cache::purge_old_global(std::time::Duration::from_secs(
                    24 * 3600,
                ));
            });
        });
        wx.app.connect_activate(|app| {
            let config = Config::load();
            build_window(app, &config);
        });
        wx
    }

    pub fn run(&self) -> i32 {
        self.app.run().value()
    }
}

fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(APP_CSS);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn build_window(app: &Application, config: &Config) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Meso")
        .default_width(config.window_width)
        .default_height(config.window_height)
        .build();

    // Shared config — panes update it; close handler saves it
    let shared_config = Rc::new(RefCell::new(config.clone()));

    // Header bar
    let header = HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some("Meso"))));

    // ── Sidebar ───────────────────────────────────────────────────────────────
    let sidebar = GBox::new(Orientation::Vertical, 0);
    sidebar.add_css_class("sidebar");
    sidebar.set_width_request(140);

    // Navigation rows
    static TABS: &[(&str, &str)] = &[
        ("radar", "Radar"),
        ("satellite", "Satellite"),
        ("alerts", "Alerts"),
        ("obs", "Obs"),
        ("spc", "SPC"),
        ("models", "Models"),
        ("soundings", "Soundings"),
        ("national", "National"),
        ("forecast", "Forecast"),
        ("text", "Text"),
    ];

    let nav_list = ListBox::new();
    nav_list.set_vexpand(true);
    nav_list.add_css_class("nav-sidebar");
    for (_id, label) in TABS {
        let row_box = GBox::new(Orientation::Horizontal, 0);
        row_box.set_margin_top(6);
        row_box.set_margin_bottom(6);
        row_box.set_margin_start(10);
        row_box.set_margin_end(6);
        let lbl = Label::new(Some(label));
        lbl.set_halign(gtk4::Align::Start);
        lbl.set_hexpand(true);
        row_box.append(&lbl);
        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));
        nav_list.append(&row);
    }
    sidebar.append(&nav_list);

    // Spacer (already done via vexpand on nav_list)
    sidebar.append(&Separator::new(Orientation::Horizontal));

    // Icon buttons at bottom: 📍 Locations  ⚙ Settings
    let icon_row = GBox::new(Orientation::Horizontal, 4);
    icon_row.set_halign(gtk4::Align::Center);
    icon_row.set_margin_top(4);
    icon_row.set_margin_bottom(4);

    let loc_btn = Button::with_label("◎");
    loc_btn.set_tooltip_text(Some("Locations"));
    loc_btn.add_css_class("flat");
    loc_btn.set_width_request(36);

    let settings_btn = Button::with_label("⚙");
    settings_btn.set_tooltip_text(Some("Settings"));
    settings_btn.add_css_class("flat");
    settings_btn.set_width_request(36);

    icon_row.append(&loc_btn);
    icon_row.append(&settings_btn);
    sidebar.append(&icon_row);

    // ── Content stack ─────────────────────────────────────────────────────────
    let stack = Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk4::StackTransitionType::None);

    // Radar pane — also returns change_site_fn for location activation
    let (radar_widget, change_site_fn) = radar_pane::build_radar_pane(Rc::clone(&shared_config));
    stack.add_named(&radar_widget, Some("radar"));

    let sat_widget = satellite_pane::build_satellite_pane(Rc::clone(&shared_config));
    stack.add_named(&sat_widget, Some("satellite"));

    let alerts_widget = alerts_pane::build_alerts_pane(Rc::clone(&shared_config));
    stack.add_named(&alerts_widget, Some("alerts"));

    let obs_widget = observations_pane::build_observations_pane(Rc::clone(&shared_config));
    stack.add_named(&obs_widget, Some("obs"));

    let spc_widget = spc_pane::build_spc_pane(Rc::clone(&shared_config));
    stack.add_named(&spc_widget, Some("spc"));

    let models_widget = models_pane::build_models_pane(Rc::clone(&shared_config));
    stack.add_named(&models_widget, Some("models"));

    let soundings_widget = soundings_pane::build_soundings_pane(Rc::clone(&shared_config));
    stack.add_named(&soundings_widget, Some("soundings"));

    let national_widget = national_pane::build_national_pane(Rc::clone(&shared_config));
    stack.add_named(&national_widget, Some("national"));

    let forecast_widget = forecast_pane::build_forecast_pane(config);
    stack.add_named(&forecast_widget, Some("forecast"));

    let radar_site = config.radar_site.clone();
    let default_wfo = meso_data::text_products::wfo_from_radar_site(&radar_site);
    let text_widget = text_pane::build_text_pane(&default_wfo, &radar_site);
    stack.add_named(&text_widget, Some("text"));

    // Select Radar tab by default
    nav_list.select_row(nav_list.row_at_index(0).as_ref());

    // Nav row selection → switch stack
    {
        let stack_c = stack.clone();
        nav_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let idx = row.index() as usize;
                if let Some((id, _)) = TABS.get(idx) {
                    stack_c.set_visible_child_name(id);
                }
            }
        });
    }

    // 📍 button → show location panel
    {
        let shared_config = Rc::clone(&shared_config);
        let change_site_fn = Rc::clone(&change_site_fn);
        let window_c = window.clone();
        loc_btn.connect_clicked(move |_| {
            let cfg = Rc::clone(&shared_config);
            let csf = Rc::clone(&change_site_fn);
            location_panel::show_location_panel(&window_c, cfg, move |site_id| {
                csf(site_id);
            });
        });
    }

    // ⚙ button → show settings panel
    {
        let shared_config = Rc::clone(&shared_config);
        let window_c = window.clone();
        settings_btn.connect_clicked(move |_| {
            settings_panel::show_settings_panel(&window_c, Rc::clone(&shared_config));
        });
    }

    // ── Assemble layout ───────────────────────────────────────────────────────
    let content = GBox::new(Orientation::Horizontal, 0);
    content.append(&sidebar);
    content.append(&Separator::new(Orientation::Vertical));
    content.append(&stack);

    let main_box = GBox::new(Orientation::Vertical, 0);
    main_box.append(&content);

    let toolbar_view = libadwaita::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&main_box));

    window.set_content(Some(&toolbar_view));

    // Save config on close
    window.connect_close_request(move |win| {
        let mut cfg = shared_config.borrow().clone();
        cfg.window_width = win.width();
        cfg.window_height = win.height();
        if let Err(e) = cfg.save() {
            tracing::warn!("Failed to save config: {e}");
        }
        glib::Propagation::Proceed
    });

    window.present();
}

// ── Application CSS ───────────────────────────────────────────────────────────

const APP_CSS: &str = r#"
/* Dark theme radar background */
.radar-background {
    background-color: #000000;
}

/* Toolbar styling */
.toolbar {
    padding: 4px;
    background-color: alpha(@window_bg_color, 0.95);
    border-bottom: 1px solid @borders;
}

/* Forecast rows */
.forecast-row {
    padding: 4px 8px;
}

/* Custom sidebar */
.sidebar {
    background-color: alpha(@window_bg_color, 0.6);
    min-width: 130px;
}

/* Sidebar nav rows */
.nav-sidebar row {
    border-radius: 4px;
    margin: 2px 4px;
}

/* Warning severity colors applied via inline CSS */

/* Active radar pane highlight (tmux-style cyan separator/border) */
.radar-pane-active {
    border: 2px solid #00d7ff;
}

.radar-pane-inactive {
    border: 1px solid alpha(#6b7280, 0.45);
}
"#;
