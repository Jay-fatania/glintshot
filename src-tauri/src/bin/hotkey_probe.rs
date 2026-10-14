//! Dev tool: reports whether global hotkeys are free on X11, without pressing
//! anything. Each combination is grabbed for an instant and released again.
//! Usage: hotkey_probe            (checks the Glintshot defaults and a few alternatives)
//!
//! A "taken" result means another X client holds that key grab. GNOME Shell's
//! own keybindings (e.g. Ctrl+Shift+Alt+R for its recorder) can intercept keys
//! without holding a plain X grab, so also check `gsettings`.

#[cfg(target_os = "linux")]
fn main() {
    use x11rb::connection::Connection;
    use x11rb::errors::ReplyError;
    use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};

    let (conn, screen_num) = x11rb::connect(None).expect("cannot connect to X server");
    let root = conn.setup().roots[screen_num].root;
    let mapping = conn
        .get_keyboard_mapping(conn.setup().min_keycode, conn.setup().max_keycode - conn.setup().min_keycode + 1)
        .unwrap()
        .reply()
        .unwrap();
    let per = mapping.keysyms_per_keycode as usize;
    let keycode_for = |keysym: u32| {
        mapping
            .keysyms
            .chunks(per)
            .position(|syms| syms.contains(&keysym))
            .map(|i| conn.setup().min_keycode + i as u8)
    };

    let ctrl_alt = ModMask::CONTROL | ModMask::M1;
    let ctrl_shift = ModMask::CONTROL | ModMask::SHIFT;
    let ctrl_alt_shift = ctrl_alt | ModMask::SHIFT;
    let super_shift = ModMask::M4 | ModMask::SHIFT;
    let combos: [(&str, ModMask, u32); 8] = [
        ("Ctrl+Alt+S", ctrl_alt, 's' as u32),
        ("Ctrl+Alt+R", ctrl_alt, 'r' as u32),
        ("Ctrl+Shift+S", ctrl_shift, 's' as u32),
        ("Ctrl+Shift+R", ctrl_shift, 'r' as u32),
        ("Ctrl+Alt+Shift+S", ctrl_alt_shift, 's' as u32),
        ("Ctrl+Alt+Shift+R", ctrl_alt_shift, 'r' as u32),
        ("Super+Shift+S", super_shift, 's' as u32),
        ("Super+Shift+R", super_shift, 'r' as u32),
    ];

    for (name, mods, keysym) in combos {
        let Some(keycode) = keycode_for(keysym) else {
            println!("{name:<18} no keycode");
            continue;
        };
        // X11 grabs match the exact modifier state, so NumLock (M2) and
        // CapsLock (LOCK) variants are separate grabs; apps usually take all four.
        let variants = [
            ("", ModMask::default()),
            ("+NumLock", ModMask::M2),
            ("+CapsLock", ModMask::LOCK),
            ("+NumLock+CapsLock", ModMask::M2 | ModMask::LOCK),
        ];
        let mut taken = Vec::new();
        for (label, extra) in variants {
            let result = conn
                .grab_key(false, root, mods | extra, keycode, GrabMode::ASYNC, GrabMode::ASYNC)
                .unwrap()
                .check();
            match result {
                Ok(()) => conn.ungrab_key(keycode, root, mods | extra).unwrap().check().unwrap(),
                Err(ReplyError::X11Error(e)) if e.error_kind == x11rb::protocol::ErrorKind::Access => {
                    taken.push(if label.is_empty() { "plain" } else { &label[1..] });
                }
                Err(e) => taken.push(Box::leak(format!("error {e:?}").into_boxed_str())),
            }
        }
        if taken.is_empty() {
            println!("{name:<18} free");
        } else {
            println!("{name:<18} TAKEN by another app ({})", taken.join(", "));
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("hotkey_probe only supports Linux X11");
}
