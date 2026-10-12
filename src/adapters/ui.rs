#[cfg(feature = "gui")]
slint::slint! {
    import { Slider, Switch, VerticalBox, HorizontalBox } from "std-widgets.slint";

    export component MainWindow inherits Window {
        title: "Shizune - Deep Virtual Mic";
        min-width: 360px;
        min-height: 220px;

        in-out property <float> attenuation_db: 100.0;
        in-out property <bool> engine_enabled: true;

        callback attenuation_changed(float);
        callback enabled_toggled(bool);

        VerticalBox {
            padding: 16px;
            spacing: 14px;

            Text {
                text: "Shizune Virtual Microphone";
                font-size: 18px;
                font-weight: 700;
                horizontal-alignment: center;
            }

            HorizontalBox {
                Text {
                    text: "Noise Suppression Active:";
                    vertical-alignment: center;
                }
                Switch {
                    checked: root.engine_enabled;
                    toggled => {
                        root.engine_enabled = self.checked;
                        root.enabled_toggled(self.checked);
                    }
                }
            }

            VerticalBox {
                spacing: 4px;
                Text {
                    text: "Attenuation Limit: " + Math.round(root.attenuation_db) + " dB";
                }
                Slider {
                    minimum: 0.0;
                    maximum: 100.0;
                    value: root.attenuation_db;
                    changed(val) => {
                        root.attenuation_db = val;
                        root.attenuation_changed(val);
                    }
                }
            }
        }
    }
}

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::adapters::cpal_source::CpalAudioSource;
use crate::domain::state::EngineState;
use crate::ports::source::AudioSource;

#[cfg(feature = "gui")]
pub fn launch_gui(
    state: Arc<EngineState>,
    source: Rc<RefCell<CpalAudioSource>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let window = MainWindow::new()?;

    window.set_attenuation_db(state.attenuation_limit.get());
    window.set_engine_enabled(state.is_enabled());

    let state_atten = Arc::clone(&state);
    window.on_attenuation_changed(move |val| {
        state_atten.attenuation_limit.set(val);
    });

    let state_enabled = Arc::clone(&state);
    window.on_enabled_toggled(move |enabled| {
        state_enabled.set_enabled(enabled);
    });

    let timer = slint::Timer::default();
    let source_clone = Rc::clone(&source);
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(500),
        move || {
            if let Ok(mut src) = source_clone.try_borrow_mut()
                && !src.is_active()
            {
                match src.reconnect_if_needed() {
                    Ok(true) => println!("[Auto-Reconnect] Microphone reconnected successfully."),
                    Ok(false) => {}
                    Err(e) => eprintln!("[Auto-Reconnect] Failed to reconnect microphone: {e}"),
                }
            }
        },
    );

    window.run()?;
    Ok(())
}

#[cfg(not(feature = "gui"))]
pub fn launch_gui(
    _state: Arc<EngineState>,
    _source: Rc<RefCell<CpalAudioSource>>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("GUI feature is disabled. Run with --features gui to enable Slint panel.");
    Ok(())
}

