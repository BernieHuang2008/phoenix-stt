use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

const CSS: &str = r#"
window.phoenix-stt-window {
    background: transparent;
}
.phoenix-capsule {
    background-color: rgba(18, 18, 22, 0.88);
    border-radius: 999px;
    padding: 8px 20px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    box-shadow: 0 4px 18px rgba(0, 0, 0, 0.35);
}
.phoenix-capsule label {
    color: #f5f5f5;
    font-size: 14px;
    font-weight: 600;
}
"#;

pub struct Ui {
    window: gtk::Window,
    label: gtk::Label,
}

impl Ui {
    pub fn build(app: &gtk::Application) -> Self {
        let window = gtk::Window::new();
        window.set_application(Some(app));
        window.add_css_class("phoenix-stt-window");
        window.set_decorated(false);
        window.set_resizable(false);

        if gtk4_layer_shell::is_supported() {
            window.init_layer_shell();
            window.set_namespace(Some("phoenix-stt"));
            window.set_layer(Layer::Overlay);
            window.set_anchor(Edge::Bottom, true);
            window.set_margin(Edge::Bottom, 64);
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_exclusive_zone(0);
        } else {
            eprintln!("phoenix-stt: 当前合成器不支持 layer-shell，使用普通窗口");
            window.set_default_size(160, 44);
            window.set_title(Some("phoenix-stt"));
        }

        let capsule = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        capsule.add_css_class("phoenix-capsule");
        capsule.set_halign(gtk::Align::Center);

        let label = gtk::Label::new(Some("录音中"));
        capsule.append(&label);
        window.set_child(Some(&capsule));

        if let Some(display) = gtk::gdk::Display::default() {
            let provider = gtk::CssProvider::new();
            provider.load_from_data(CSS);
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        window.hide();

        Self { window, label }
    }

    pub fn show(&self, text: &str) {
        self.label.set_text(text);
        self.window.present();
    }

    pub fn hide(&self) {
        self.window.hide();
    }
}
