mod core;

use core::cell::{Cell, CellMeta};

fn main() {
    let cell = Cell::new(
        1,
        CellMeta::with_description("PrimaryCell", "The first cell of the HubFlow system."),
    );

    println!("{cell}");
    println!("  UUID     : {}", cell.id());
    println!("  local_id : {}", cell.local_id());
    println!("  name     : {}", cell.meta().name);
    println!(
        "  desc     : {}",
        cell.meta().description.as_deref().unwrap_or("<none>")
    );
}
