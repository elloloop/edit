use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn translate_key_event(key: KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => Some(vec![3]),
            KeyCode::Char('d') | KeyCode::Char('D') => Some(vec![4]),
            KeyCode::Char('l') | KeyCode::Char('L') => Some(vec![12]),
            KeyCode::Char('z') | KeyCode::Char('Z') => Some(vec![26]),
            _ => None,
        };
    }

    let arrow = |normal: &'static [u8], app: &'static [u8]| -> Vec<u8> {
        if application_cursor {
            app.to_vec()
        } else {
            normal.to_vec()
        }
    };

    match key.code {
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(arrow(b"\x1b[A", b"\x1bOA")),
        KeyCode::Down => Some(arrow(b"\x1b[B", b"\x1bOB")),
        KeyCode::Right => Some(arrow(b"\x1b[C", b"\x1bOC")),
        KeyCode::Left => Some(arrow(b"\x1b[D", b"\x1bOD")),
        KeyCode::Home => Some(vec![0x1b, b'[', b'H']),
        KeyCode::End => Some(vec![0x1b, b'[', b'F']),
        KeyCode::Insert => Some(vec![0x1b, b'[', b'2', b'~']),
        KeyCode::Delete => Some(vec![0x1b, b'[', b'3', b'~']),
        KeyCode::PageUp => Some(vec![0x1b, b'[', b'5', b'~']),
        KeyCode::PageDown => Some(vec![0x1b, b'[', b'6', b'~']),
        KeyCode::Char(ch) => Some(ch.to_string().into_bytes()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::translate_key_event;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn ctrl_shortcuts_translate_to_control_bytes() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(translate_key_event(key, false), Some(vec![3]));

        let key = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(translate_key_event(key, false), Some(vec![4]));
    }

    #[test]
    fn arrow_keys_respect_application_cursor_mode() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(translate_key_event(key, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(translate_key_event(key, true), Some(b"\x1bOA".to_vec()));
    }
}
