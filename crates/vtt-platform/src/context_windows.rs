//! Lecture du texte autour du curseur sous Windows via UI Automation
//! (`IUIAutomation`), appelée uniquement lorsque l'utilisateur a activé
//! l'option opt-in `dictation.screen_context`.
//!
//! Sécurité :
//! - vérifie `CurrentIsPassword()` AVANT toute lecture de texte et abandonne
//!   aussitôt si le champ est un mot de passe ;
//! - borne la lecture à ~2 000 caractères autour du curseur (4 000 max) ;
//! - ne journalise jamais le texte lu.

use vtt_core::context::MAX_RAW_CONTEXT_CHARS;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationValuePattern, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
    TextUnit_Character, UIA_TextPatternId, UIA_ValuePatternId,
};

use crate::FocusedField;

/// Rayon de lecture autour du curseur (caractères avant et après).
const CARET_RADIUS_CHARS: i32 = 1_000;

struct ComGuard(bool);

impl ComGuard {
    fn init() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self(hr.is_ok())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Lit le champ ayant le focus clavier via UI Automation.
/// Retourne `FocusedField::default()` si la fenêtre active n'expose aucun
/// texte accessible ou si Windows refuse l'accès.
pub fn focused_field_context() -> FocusedField {
    let _com = ComGuard::init();
    let Ok(uia) = (unsafe {
        CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
    }) else {
        return FocusedField::default();
    };
    let Ok(element) = (unsafe { uia.GetFocusedElement() }) else {
        return FocusedField::default();
    };
    read_element(&element)
}

fn read_element(element: &IUIAutomationElement) -> FocusedField {
    // Vérification stricte AVANT toute lecture de texte.
    if unsafe { element.CurrentIsPassword() }.is_ok_and(|b| b.as_bool()) {
        return FocusedField {
            text: None,
            is_password: true,
        };
    }

    let text = read_text_pattern(element)
        .or_else(|| read_value_pattern(element))
        .filter(|s| !s.trim().is_empty());

    FocusedField {
        text,
        is_password: false,
    }
}

fn read_text_pattern(element: &IUIAutomationElement) -> Option<String> {
    let pattern: IUIAutomationTextPattern =
        unsafe { element.GetCurrentPatternAs(UIA_TextPatternId) }.ok()?;
    let max_chars = MAX_RAW_CONTEXT_CHARS as i32;

    if let Ok(selection) = unsafe { pattern.GetSelection() }
        && unsafe { selection.Length() }.unwrap_or(0) > 0
        && let Ok(range) = unsafe { selection.GetElement(0) }
        && let Ok(window) = unsafe { range.Clone() }
    {
        let _ = unsafe {
            window.MoveEndpointByUnit(
                TextPatternRangeEndpoint_Start,
                TextUnit_Character,
                -CARET_RADIUS_CHARS,
            )
        };
        let _ = unsafe {
            window.MoveEndpointByUnit(
                TextPatternRangeEndpoint_End,
                TextUnit_Character,
                CARET_RADIUS_CHARS,
            )
        };
        if let Ok(bstr) = unsafe { window.GetText(max_chars) } {
            let text = bstr.to_string();
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }

    let doc = unsafe { pattern.DocumentRange() }.ok()?;
    let bstr = unsafe { doc.GetText(max_chars) }.ok()?;
    Some(bstr.to_string())
}

fn read_value_pattern(element: &IUIAutomationElement) -> Option<String> {
    let pattern: IUIAutomationValuePattern =
        unsafe { element.GetCurrentPatternAs(UIA_ValuePatternId) }.ok()?;
    let bstr = unsafe { pattern.CurrentValue() }.ok()?;
    let full = bstr.to_string();
    if full.chars().count() <= MAX_RAW_CONTEXT_CHARS {
        return Some(full);
    }
    let keep: String = full.chars().take(MAX_RAW_CONTEXT_CHARS).collect();
    Some(keep)
}
