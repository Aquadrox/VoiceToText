//! Essai manuel de traduction locale M2M100 (ONNX Runtime).
//!
//! ```
//! cargo run --release -p vtt-asr --example translate_text -- <dossier-modele> <libonnxruntime> <langue> <texte>
//! ```

use std::path::PathBuf;
use std::time::Instant;

use vtt_asr::M2m100Translator;
use vtt_core::Translator;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("dossier du modèle M2M100"));
    let runtime = PathBuf::from(args.next().expect("chemin libonnxruntime"));
    let target_lang = args
        .next()
        .expect("langue cible (ex. anglais, japonais, es)");
    let text = args.collect::<Vec<_>>().join(" ");
    assert!(!text.is_empty(), "texte à traduire requis");

    let translator = M2m100Translator::new(
        dir.join("encoder_model_quantized.onnx"),
        dir.join("decoder_model_merged_quantized.onnx"),
        dir.join("tokenizer.json"),
        runtime,
        4,
    );

    let t0 = Instant::now();
    let translated = translator
        .translate(&text, "fr", &target_lang)
        .expect("échec traduction");
    println!(
        "[{target_lang}] ({} ms) {translated}",
        t0.elapsed().as_millis()
    );
}
