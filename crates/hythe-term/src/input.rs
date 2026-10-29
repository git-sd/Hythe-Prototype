//! Turns key presses and pastes into the bytes a terminal application expects.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    fn any(self) -> bool {
        self.ctrl || self.alt || self.shift
    }

    /// xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4).
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// Function key, 1-12.
    F(u8),
}

/// Bytes to send for `key`, or `None` if it has no terminal encoding.
/// `app_cursor` is the terminal's application-cursor-keys mode.
pub fn encode_key(key: &Key, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    let csi_letter = |letter: char| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[1;{}{}", mods.param(), letter).into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let csi_tilde = |n: u8| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[{};{}~", n, mods.param()).into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    Some(match key {
        Key::Char(c) => {
            let mut out = Vec::new();
            if mods.alt {
                out.push(0x1b);
            }
            if mods.ctrl {
                let byte = match c {
                    'a'..='z' => *c as u8 - b'a' + 1,
                    'A'..='Z' => *c as u8 - b'A' + 1,
                    ' ' | '@' | '2' => 0,
                    '[' | '3' => 27,
                    '\\' | '4' => 28,
                    ']' | '5' => 29,
                    '^' | '6' => 30,
                    '_' | '7' | '-' => 31,
                    '?' | '8' => 127,
                    _ => return None,
                };
                out.push(byte);
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            out
        }
        // Shift+Enter sends a bare line feed, which agent CLIs treat as "newline, don't submit".
        Key::Enter if mods.shift => b"\n".to_vec(),
        Key::Enter if mods.alt => b"\x1b\r".to_vec(),
        Key::Enter => b"\r".to_vec(),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Backspace if mods.ctrl => vec![0x08],
        Key::Backspace if mods.alt => vec![0x1b, 0x7f],
        Key::Backspace => vec![0x7f],
        Key::Escape => vec![0x1b],
        Key::Up => csi_letter('A'),
        Key::Down => csi_letter('B'),
        Key::Right => csi_letter('C'),
        Key::Left => csi_letter('D'),
        Key::Home => csi_letter('H'),
        Key::End => csi_letter('F'),
        Key::Insert => csi_tilde(2),
        Key::Delete => csi_tilde(3),
        Key::PageUp => csi_tilde(5),
        Key::PageDown => csi_tilde(6),
        Key::F(n @ 1..=4) => {
            let letter = ['P', 'Q', 'R', 'S'][*n as usize - 1];
            if mods.any() {
                format!("\x1b[1;{}{}", mods.param(), letter).into_bytes()
            } else {
                format!("\x1bO{letter}").into_bytes()
            }
        }
        Key::F(n @ 5..=12) => {
            let code = [15, 17, 18, 19, 20, 21, 23, 24][*n as usize - 5];
            csi_tilde(code)
        }
        Key::F(_) => return None,
    })
}

/// Bytes for pasted text. With bracketed paste the application can tell a
/// paste from typing, which matters for multi-line input to agent CLIs.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    // Normalise line endings the way a terminal would receive Enter.
    let body = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // Strip an embedded end marker so pasted text can't break out of the bracket.
        let safe = body.replace("\x1b[201~", "");
        format!("\x1b[200~{safe}\x1b[201~").into_bytes()
    } else {
        body.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { ctrl: false, alt: false, shift: false };
    const CTRL: Mods = Mods { ctrl: true, alt: false, shift: false };
    const ALT: Mods = Mods { ctrl: false, alt: true, shift: false };
    const SHIFT: Mods = Mods { ctrl: false, alt: false, shift: true };

    fn enc(key: Key, mods: Mods, app: bool) -> Vec<u8> {
        encode_key(&key, mods, app).unwrap()
    }

    #[test]
    fn plain_and_control_characters() {
        assert_eq!(enc(Key::Char('a'), NONE, false), b"a");
        assert_eq!(enc(Key::Char('é'), NONE, false), "é".as_bytes());
        assert_eq!(enc(Key::Char('c'), CTRL, false), vec![3]);
        assert_eq!(enc(Key::Char('d'), CTRL, false), vec![4]);
        assert_eq!(enc(Key::Char('['), CTRL, false), vec![27]);
        assert_eq!(enc(Key::Char('b'), ALT, false), vec![0x1b, b'b']);
        assert!(encode_key(&Key::Char('1'), CTRL, false).is_none());
    }

    #[test]
    fn arrows_respect_application_mode_and_modifiers() {
        assert_eq!(enc(Key::Up, NONE, false), b"\x1b[A");
        assert_eq!(enc(Key::Up, NONE, true), b"\x1bOA");
        assert_eq!(enc(Key::Left, CTRL, false), b"\x1b[1;5D");
        assert_eq!(enc(Key::Right, SHIFT, true), b"\x1b[1;2C");
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(enc(Key::Enter, NONE, false), b"\r");
        assert_eq!(enc(Key::Enter, SHIFT, false), b"\n");
        assert_eq!(enc(Key::Enter, ALT, false), b"\x1b\r");
        assert_eq!(enc(Key::Backspace, NONE, false), vec![0x7f]);
        assert_eq!(enc(Key::Tab, SHIFT, false), b"\x1b[Z");
        assert_eq!(enc(Key::Delete, NONE, false), b"\x1b[3~");
        assert_eq!(enc(Key::PageUp, CTRL, false), b"\x1b[5;5~");
        assert_eq!(enc(Key::F(1), NONE, false), b"\x1bOP");
        assert_eq!(enc(Key::F(5), NONE, false), b"\x1b[15~");
        assert_eq!(enc(Key::F(12), SHIFT, false), b"\x1b[24;2~");
        assert!(encode_key(&Key::F(13), NONE, false).is_none());
    }

    #[test]
    fn paste_is_bracketed_and_sanitised() {
        assert_eq!(encode_paste("a\nb", false), b"a\rb");
        assert_eq!(encode_paste("a\r\nb", true), b"\x1b[200~a\rb\x1b[201~");
        assert_eq!(encode_paste("x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~");
    }
}
