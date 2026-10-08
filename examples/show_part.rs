use mbirasyth::compose::{MusicParams, Piece, PULSES};
use mbirasyth::tuning::{degree_letter, KEYS};
fn main() {
    let seed: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let var: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let piece = Piece::generate(&MusicParams { seed, ..Default::default() });
    let name = |k: u8| { let d = KEYS[k as usize].degree; format!("{}{}", degree_letter(d), d / 7) };
    let c = &piece.kushaura[var];
    println!("variation {var}");
    for (label, right) in [("right", true), ("left ", false)] {
        print!("{label}: ");
        for p in 0..PULSES {
            if p % 6 == 0 { print!("| "); }
            let n = c.notes[p].iter().flatten().find(|n| KEYS[n.key as usize].manual.is_left() != right && n.importance >= 0.3);
            print!("{:3}", n.map(|n| name(n.key)).unwrap_or(" .".into()));
        }
        println!();
    }
    print!("chord: ");
    for p in 0..PULSES { if p % 6 == 0 { print!("| "); } print!("{:3}", if p % 4 == 0 { degree_letter(piece.roots[p / 4]).to_string() } else { "".into() }); }
    println!();
}
