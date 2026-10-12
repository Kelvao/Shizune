#[cfg(feature = "gui")]
slint::slint! {
    import { ComboBox, Slider, Switch, VerticalBox, HorizontalBox } from "std-widgets.slint";

    export component MainWindow inherits Window {
        title: "Shizune - Deep Virtual Mic";
        min-width: 400px;
        min-height: 310px;

        in-out property <float> attenuation_db: 100.0;
        in-out property <bool> engine_enabled: true;
        in-out property <[string]> input_devices: ["Default System Microphone"];
        in-out property <string> selected_device: "Default System Microphone";

        callback attenuation_changed(float);
        callback enabled_toggled(bool);
        callback device_selected(string);

        VerticalBox {
            padding: 16px;
            spacing: 12px;

            Text {
                text: "Shizune Virtual Microphone";
                font-size: 18px;
                font-weight: 700;
                horizontal-alignment: center;
            }

            Text {
                text: "Filtered audio is routed to: Shizune_Virtual_Mic";
                font-size: 11px;
                color: #718096;
                horizontal-alignment: center;
            }

            VerticalBox {
                spacing: 4px;
                Text {
                    text: "Select Input Microphone to Filter:";
                    font-size: 13px;
                    font-weight: 600;
                }
                ComboBox {
                    model: root.input_devices;
                    current-value: root.selected_device;
                    selected(val) => {
                        root.selected_device = val;
                        root.device_selected(val);
                    }
                }
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

use cpal::traits::{DeviceTrait, HostTrait};

use crate::adapters::cpal_source::CpalAudioSource;
use crate::domain::state::EngineState;
use crate::ports::source::AudioSource;

#[cfg(feature = "gui")]
pub fn launch_gui(
    state: Arc<EngineState>,
    source: Rc<RefCell<CpalAudioSource>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let window = MainWindow::new()?;

    // Populate physical input devices list
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());
    let default_label = match default_name {
        Some(ref name) => format!("Default System Microphone ({name})"),
        None => "Default System Microphone".to_string(),
    };

    let available_devices = CpalAudioSource::list_input_devices().unwrap_or_default();
    let mut dev_list: Vec<slint::SharedString> = Vec::new();
    dev_list.push(default_label.as_str().into());
    for dev in available_devices {
        dev_list.push(dev.into());
    }
    let model = std::rc::Rc::new(slint::VecModel::from(dev_list));
    window.set_input_devices(model.into());
    window.set_selected_device(default_label.as_str().into());

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

    let source_for_dev = Rc::clone(&source);
    window.on_device_selected(move |val| {
        let selected_str = val.to_string();
        let target = if selected_str.starts_with("Default System Microphone") {
            None
        } else {
            Some(selected_str.clone())
        };
        println!("[Audio Input] Selected physical microphone: {selected_str}");
        if let Ok(mut src) = source_for_dev.try_borrow_mut() {
            if let Err(e) = src.switch_device(target) {
                eprintln!("[Warning] Failed to switch microphone: {e}");
            } else {
                println!("[Audio Input] Switched input to: {selected_str} -> Filtering into Shizune_Virtual_Mic");
            }
        }
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

