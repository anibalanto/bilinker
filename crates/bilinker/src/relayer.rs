//! `bilinker relayer` — mover los bilinks de una capa a la de arriba.
//!
//! Existe por un modo de falla concreto: **un `.bilink/` fabrica una raíz de capa**,
//! porque es uno de los marcadores con los que bilinker resuelve la raíz. Si queda
//! en un directorio que stratum no declara como capa, las dos herramientas
//! discrepan sobre dónde termina una — y el `check` de la capa de arriba deja de
//! ver esos bilinks **sin decir nada**, que es la peor forma de no verlos.
//!
//! **Lo que se mueve es la ubicación, nunca el contenido.** El id de un capture es
//! `sha256(file \0 query \0)`, así que prefijar el `file` le cambia el id; los
//! `hash` no se tocan y ningún endpoint pasa a `ALTERED`. Es lo mismo que hacen
//! `apply` y `accept --place` juntos, pero entre capas.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use bilink_format::bilink::bilink_files;
use bilink_format::{BiLink, Capture, LinkEndpoint};

pub struct RelayerResult {
    /// La capa que se vacía, relativa a la de destino.
    pub layer: String,
    pub captures: usize,
    pub bilinks: usize,
    /// Bilinks de otras capas cuyo `accepted.link` copiaba un id que cambió.
    pub neighbours: usize,
}

/// Mueve `<destino>/<layer>/.bilink/` a `<destino>/.bilink/`.
pub fn relayer(dest: &Path, layer: &str, dry_run: bool) -> Result<RelayerResult> {
    let src = dest.join(layer);
    let src_bl = src.join(".bilink");
    if !src_bl.is_dir() {
        bail!("{layer} no tiene .bilink/ propio: no hay capa que fusionar");
    }
    if dest.join(".bilink").canonicalize().ok() == src_bl.canonicalize().ok() {
        bail!("{layer} es la capa de destino");
    }

    // ── los captures se reacuñan con el `file` prefijado ─────────────────────
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    let mut acuñados: Vec<(String, Capture)> = Vec::new();

    for (viejo, cap) in captures_in(&src_bl)? {
        let movido = Capture { file: format!("{layer}/{}", cap.file), ..cap };
        let nuevo = movido.id();
        renames.insert(viejo, nuevo.clone());
        acuñados.push((nuevo, movido));
    }

    // ── los bilinks de la capa, a la de arriba ───────────────────────────────
    let mut movidos: Vec<(PathBuf, BiLink)> = Vec::new();
    for path in bilink_files(&src_bl) {
        let mut bl = BiLink::load(&path)?;
        for n in [0u8, 1u8] {
            let e = bl.endpoint.get_mut(n);
            e.link = moved_link(&e.link, &renames, layer)?;
            // **Todas las entradas**, no la primera: un `relayer` mueve la capa
            // entera, y una decisión desplazada apunta a un capture que se movió
            // igual que las demás.
            for a in e.accepted.iter_mut() {
                if let Some(l) = a.link.take() {
                    // **Un `accepted.link` de un endpoint `path` es una copia opaca
                    // del id del vecino, y ese capture no se movió**: sólo se
                    // reescribe si el id está en el mapa de renames.
                    a.link = Some(moved_link(&l, &renames, layer)?);
                }
            }
        }
        let nombre = path.file_name().context("el bilink no tiene nombre")?;
        movidos.push((dest.join(".bilink").join(nombre), bl));
    }

    // ── los vecinos que copiaban un id que cambió ────────────────────────────
    let mut vecinos = Vec::new();
    for path in neighbour_bilinks(&src)? {
        let mut bl = BiLink::load(&path)?;
        let mut tocado = false;
        for n in [0u8, 1u8] {
            for a in bl.endpoint.get_mut(n).accepted.iter_mut() {
                let Some(LinkEndpoint::Capture(id)) = a.link.as_ref() else { continue };
                if let Some(nuevo) = renames.get(id.as_str()) {
                    a.link = Some(format!("capture {nuevo}").parse()?);
                    tocado = true;
                }
            }
        }
        if tocado {
            vecinos.push((path, bl));
        }
    }

    let r = RelayerResult {
        layer: layer.to_string(),
        captures: acuñados.len(),
        bilinks: movidos.len(),
        neighbours: vecinos.len(),
    };
    if dry_run {
        return Ok(r);
    }

    // ── recién acá se escribe ────────────────────────────────────────────────
    for (id, cap) in &acuñados {
        let p = Capture::dir(dest).join(format!("{id}.yaml"));
        if let Some(d) = p.parent() { std::fs::create_dir_all(d)?; }
        std::fs::write(&p, serde_yaml_ng::to_string(cap)?)?;
    }
    for (p, bl) in &movidos {
        bl.write(p)?;
    }
    for (p, bl) in &vecinos {
        bl.write(p)?;
    }
    std::fs::remove_dir_all(&src_bl)?;

    Ok(r)
}

/// El `link` de un endpoint, visto desde la capa de arriba.
///
/// Un `capture` cambia de id; un `path` que era relativo a la capa que se vacía
/// necesita el prefijo. `issue`, `repo` y `abstract` no dependen de la capa.
fn moved_link(
    link: &LinkEndpoint,
    renames: &BTreeMap<String, String>,
    layer: &str,
) -> Result<LinkEndpoint> {
    Ok(match link {
        LinkEndpoint::Capture(id) => match renames.get(id.as_str()) {
            Some(nuevo) => format!("capture {nuevo}").parse()?,
            None => link.clone(),
        },
        LinkEndpoint::Path(p) => {
            let escrito = p.to_string();
            // `>impl` era relativo a la capa que se vacía; desde arriba es
            // `<layer>>impl`. Un path que ya arranca con `*` o `<` no es relativo a
            // ella y no se toca.
            if escrito.starts_with('>') {
                format!("path {layer}{escrito}").parse()?
            } else {
                link.clone()
            }
        }
        _ => link.clone(),
    })
}

fn captures_in(bl_dir: &Path) -> Result<Vec<(String, Capture)>> {
    let dir = bl_dir.join("capture");
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(out) };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "yaml") {
            continue;
        }
        let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        let cap: Capture = serde_yaml_ng::from_str(&std::fs::read_to_string(&p)?)
            .with_context(|| format!("leyendo {}", p.display()))?;
        out.push((id, cap));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Los bilinks de las capas que cuelgan de la que se vacía — las que pudieron
/// copiar un id suyo en un `accepted.link`.
fn neighbour_bilinks(src: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let stratum = src.join(".stratum");
    let Ok(rd) = std::fs::read_dir(&stratum) else { return Ok(out) };
    for e in rd.flatten() {
        let bl = e.path().join(".bilink");
        if bl.is_dir() {
            out.extend(bilink_files(&bl));
        }
    }
    Ok(out)
}
