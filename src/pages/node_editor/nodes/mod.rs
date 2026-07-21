//! Тела (body builders) для функциональных нод node-editor'а.
//!
//! Каждый под-модуль реализует `pub fn body(node: &NodeInstance) -> Box<dyn Widget>`,
//! который регистрируется в `registry::NodeKindMeta::body`. Header и port-rows
//! рисуются стандартным `node_view`, body замещает дефолтные field-rows.
//!
//! Для long-lived state (плеер, рекордер, загруженный файл) используется
//! `NodeInstance.runtime: Arc<Mutex<NodeRuntime>>` — см. `types::NodeRuntime`.

pub mod acestep;
pub mod asr_gigaam;
pub mod audio_equalizer;
pub mod audio_file;
pub mod audio_filter;
pub mod audio_gain;
pub mod audio_mixer;
pub mod audio_player;
pub mod audio_recorder;
pub mod audio_reverb;
pub mod audio_save;
pub mod decode;
pub mod ffmpeg_player;
pub mod llm;
pub mod ltx;
pub mod markdown_view;
pub mod omnivoice;
pub mod scalar;
pub mod sortformer_diarizer;
pub mod text_view;
pub mod voxcpm2;
