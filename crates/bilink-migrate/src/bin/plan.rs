//! Muestra qué haría la migración sobre una capa, sin escribir nada.

fn main() -> anyhow::Result<()> {
    let layer = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let layer = std::path::Path::new(&layer);

    let p = bilink_migrate::partition::plan(layer)?;
    println!("{}", p.summary(layer));

    let problems = bilink_migrate::partition::verify(layer)?;
    if problems.is_empty() {
        println!("  verificación: ningún hash aceptado se pierde");
    } else {
        for x in &problems { println!("  PROBLEMA  {x}"); }
        std::process::exit(1);
    }
    Ok(())
}
