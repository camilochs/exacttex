//! Prints what the scanner makes of a file.
use nextex_core::scanner::{Piece, scan};

fn main() {
    let path = std::env::args().nth(1).expect("usage: pieces <file>");
    let bytes = std::fs::read(&path).expect("readable");
    for piece in scan(&bytes) {
        piece.walk(&mut |piece| {
            let (label, span) = match piece {
                Piece::Text(s) => ("text", *s),
                Piece::Excluded(s) => ("excluded", *s),
                Piece::Construct { kind, span, .. } => (kind.name(), *span),
                Piece::Malformed { kind, span } => {
                    println!(
                        "  MALFORMED {:<10} {:?}",
                        kind.name(),
                        String::from_utf8_lossy(
                            &bytes[span.start()..span.end().min(span.start() + 40)]
                        )
                    );
                    return;
                }
            };
            if label != "text" {
                println!(
                    "  {:<10} {:?}",
                    label,
                    String::from_utf8_lossy(
                        &bytes[span.start()..span.end().min(span.start() + 46)]
                    )
                );
            }
        });
    }
}
