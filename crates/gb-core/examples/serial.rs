//! Lance une ROM sans écran et affiche ce qu'elle envoie sur le port série.
//!
//! Les ROMs de test CPU open source écrivent leurs résultats sur ce port :
//! c'est le moyen le plus simple de valider le CPU avant d'avoir un écran.
//!
//! ```text
//! cargo run --release -p gb-core --example serial -- chemin/vers/test.gb [secondes]
//! ```

use std::io::Write;
use std::process::ExitCode;

use emu_core::Emulator;
use gb_core::GameBoy;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("Utilisation : serial <rom.gb> [secondes émulées, 60 par défaut]");
        return ExitCode::FAILURE;
    };
    let seconds: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(60);

    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("Impossible de lire {path} : {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut gb = GameBoy::new();
    if let Err(err) = gb.load_rom(&data) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }

    let mut printed = 0;
    let mut stdout = std::io::stdout();
    for _ in 0..seconds * 60 {
        gb.run_frame();
        let out = gb.serial_output();
        if out.len() > printed {
            let _ = stdout.write_all(&out[printed..]);
            let _ = stdout.flush();
            printed = out.len();
        }
        let text = String::from_utf8_lossy(out);
        if text.contains("Passed") || text.contains("Failed") {
            break;
        }
    }
    println!();

    if gb.cpu().locked {
        eprintln!("Le CPU s'est bloqué sur un opcode illégal.");
    }
    if String::from_utf8_lossy(gb.serial_output()).contains("Passed") {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
