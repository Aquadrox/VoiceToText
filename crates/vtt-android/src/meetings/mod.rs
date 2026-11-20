//! Réunions sur le téléphone : enregistrement au micro ou import d'un
//! fichier audio, puis, à la fin, transcription par blocs avec le moteur de
//! la dictée et compte rendu par le LLM.
//!
//! Une réunion par dossier (`meetings/<id>`), au même format que sur le PC
//! (`meeting.json`, [`vtt_core::meeting::MeetingRecord`]). L'audio, en PCM
//! 16 bits 16 kHz mono, va selon le choix de l'utilisateur dans le stockage
//! privé de l'application (chiffré par Android, inaccessible aux autres
//! applications) ou en mémoire seulement.
//!
//! L'application Kotlin capte ou décode l'audio et le transmet tel quel
//! ([`feed`]) : la conversion en 16 kHz mono est faite ici.

mod capture;
mod process;
mod storage;
#[cfg(test)]
mod tests;
mod views;

pub use capture::{New, create, feed, mark, receiving, recover, set_progress, stop};
pub use process::{
    clear_voicebook, enroll_speaker_voice, forget_speakers, identify, process,
    remove_voicebook_entry, rename_speaker, rename_voicebook_entry, report, report_html,
    set_action_done, set_speaker_count, summarize, voicebook_list,
};
pub use views::{
    Detail, Reply, ReportView, Row, SpeakerView, SummaryView, clip, delete, detail, export, list,
    rename, set_conference,
};
