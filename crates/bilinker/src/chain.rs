use std::path::{Component, Path, PathBuf};
use anyhow::{bail, Context, Result};
use uuid::Uuid;

use bilink_format::{BiLink, LinkEndpoint};

pub struct ChainNew {
    pub uuid: String,
    pub files: Vec<PathBuf>,
}

/// Los campos de declaración de una cadena nueva.
///
/// Son inertes —no entran en ningún hash ni en ningún estado— y por eso viajan
/// aparte de los tips: no cambian qué se compara, sólo qué dice el vínculo.
///
/// Van **sólo en los tips**. Un mid es un tramo de la cadena y no una punta: no
/// tiene rol semántico que etiquetar, y darle uno inventaría un dato que nadie
/// declaró. El `kind` sí va en todos, porque clasifica la relación entera.
#[derive(Default)]
pub struct Declaration {
    pub kind: Option<String>,
    /// El `name` de cada tip, en el orden en que se pasaron.
    pub name: [Option<String>; 2],
    /// El nombre del generador que capturó cada tip: lo que dijo `--as.N`.
    ///
    /// Va acá y no adentro del tip porque **es declaración y no ubicación**: el
    /// generador ya hizo lo suyo cuando este struct llega, y lo que queda es dejar
    /// escrito con qué se hizo. Sin `--as`, ninguno — que es lo que dice un capture
    /// del núcleo, y lo que dice todo archivo escrito antes de este campo.
    pub r#as: [Option<String>; 2],
    /// El uuid del bilink remoto, cuando un tip es `repo`. En una cadena local se
    /// genera; cruzando la frontera se toma del proveedor, porque **el uuid
    /// compartido es el rendezvous**.
    pub uuid: Option<String>,
}

/// Creates a new chain or direct link.
///
/// `tips`: exactly 2 entries of (layer_path_relative_to_root, structural_endpoint).
/// `mids`: ordered layer paths between the two tips.
/// All paths are relative to `root`.
/// Crea una cadena. Con un tip `repo`, el uuid viene de afuera y no se genera.
pub fn chain_new(
    root: &Path,
    tips: &[(PathBuf, LinkEndpoint)],
    mids: &[PathBuf],
    decl: &Declaration,
) -> Result<ChainNew> {
    if tips.len() != 2 {
        bail!("chain new requires exactly 2 --tip arguments");
    }

    // **El UUID es el rendezvous.** Del lado del consumidor no se genera: se toma el
    // del bilink que el proveedor publicó, porque la convención de UUID compartido
    // es lo que hace que los dos lados se encuentren sin que ninguno escriba en el
    // repo del otro. Generar uno propio rompería el vínculo antes de crearlo.
    let uuid = match tips.iter().find_map(|(_, ep)| ep.repo_alias()) {
        Some(_) => decl.uuid.clone().context(
            "un tip `repo` necesita el uuid del bilink remoto: `--from-repo <alias>:<uuid>`",
        )?,
        None => Uuid::new_v4().to_string(),
    };

    let all_layers: Vec<PathBuf> = {
        let mut v = vec![tips[0].0.clone()];
        v.extend_from_slice(mids);
        v.push(tips[1].0.clone());
        v
    };

    let n = all_layers.len();
    let mut created = Vec::new();

    // Same-layer direct link: both tips in the same directory → one file.
    if n == 2 && normalize(&all_layers[0]) == normalize(&all_layers[1]) {
        let mut bl = BiLink::new(tips[0].1.clone(), tips[1].1.clone());
        bl.kind = decl.kind.clone();
        bl.endpoint.get_mut(0).name = decl.name[0].clone();
        bl.endpoint.get_mut(1).name = decl.name[1].clone();
        bl.endpoint.get_mut(0).r#as = decl.r#as[0].clone();
        bl.endpoint.get_mut(1).r#as = decl.r#as[1].clone();
        let path = bilink_path(root, &all_layers[0], &uuid);
        bl.write(&path)?;
        created.push(path);
        return Ok(ChainNew { uuid, files: created });
    }

    // Multi-layer chain
    for i in 0..n {
        let layer = &all_layers[i];

        let (link0, link1) = if i == 0 {
            let to_next = layer_endpoint(layer, &all_layers[i + 1])?;
            (tips[0].1.clone(), to_next)
        } else if i == n - 1 {
            let to_prev = layer_endpoint(layer, &all_layers[i - 1])?;
            (to_prev, tips[1].1.clone())
        } else {
            let to_prev = layer_endpoint(layer, &all_layers[i - 1])?;
            let to_next = layer_endpoint(layer, &all_layers[i + 1])?;
            (to_prev, to_next)
        };

        let mut bl = BiLink::new(link0, link1);
        bl.kind = decl.kind.clone();
        // El `name` de un tip viaja con el endpoint estructural, que es el 0 en el
        // primer nodo y el 1 en el último. Los mids no llevan ninguno.
        //
        // El `as` viaja igual, y por una razón más fuerte: dice con qué se capturó
        // un fragmento, y un mid no captura ninguno.
        if i == 0        { bl.endpoint.get_mut(0).name = decl.name[0].clone();
                           bl.endpoint.get_mut(0).r#as = decl.r#as[0].clone(); }
        if i == n - 1    { bl.endpoint.get_mut(1).name = decl.name[1].clone();
                           bl.endpoint.get_mut(1).r#as = decl.r#as[1].clone(); }
        let path = bilink_path(root, layer, &uuid);
        bl.write(&path)?;
        created.push(path);
    }

    Ok(ChainNew { uuid, files: created })
}

pub fn resolve_layer_link(
    bilink_file: &Path,
    layer_root: &Path,
    link_path: &Path,
    uuid: &str,
) -> PathBuf {
    let _ = bilink_file;
    BiLink::path_in(&layer_root.join(link_path), uuid)
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// El path del bilink en una capa, **dejando la capa bien formada**.
///
/// Crear un bilink es crear el `.bilink/` si no estaba, y un `.bilink/` sin su
/// `.gitignore` y sin su `version` está a medias: el primero deja que se commiteen
/// derivados, y el segundo lo vuelve indistinguible de una capa anterior a que el
/// campo existiera — que del otro lado de la frontera significa "no puedo
/// interpretar lo que publica".
fn bilink_path(root: &Path, layer: &Path, uuid: &str) -> PathBuf {
    let layer_root = root.join(layer);
    let _ = bilink_format::write_ignore(&layer_root);
    let _ = bilink_format::ensure_version(&layer_root);
    BiLink::path_in(&layer_root, uuid)
}

/// El endpoint `path` que, parado en `from_layer`, nombra a `to_layer`.
///
/// **No es el path relativo del filesystem.** `<` no sube dos componentes: sube un
/// nivel de capa y de ahí camina hasta la raíz verdadera de esa capa —el `.git` o
/// `.bilink` que la delimita—, atravesando los directorios comunes que haya en el
/// medio. Contar `../..` de a pares, que es lo que hace un diff de paths, toma
/// `subsystems/bilinker` por un nivel de capa y produce `<<` donde va `<`.
///
/// Así que se cuentan capas: tantos `<` como niveles tenga `from_layer` —hasta la
/// raíz— y de ahí el descenso a `to_layer`.
fn layer_endpoint(from_layer: &Path, to_layer: &Path) -> Result<LinkEndpoint> {
    use stratum::PathToken;

    let ups = normalize(from_layer).components()
        .filter(|c| matches!(c, Component::Normal(n) if *n == std::ffi::OsStr::new(".stratum")))
        .count();

    let mut tokens: stratum::StratumPath = std::iter::repeat(PathToken::Up).take(ups).collect();
    let down = normalize(to_layer);
    if down.components().next().is_some() {
        tokens.extend(filesystem_to_stratum_tokens(&down)?);
    }
    if tokens.is_empty() {
        bail!("no hay path de capa entre '{}' y '{}'", from_layer.display(), to_layer.display());
    }
    format!("path {}", stratum::format_path(&tokens)).parse()
}

/// Convierte un path de filesystem —siempre descendente— en tokens Stratum.
///
/// Alterna dos formas sin orden fijo: `.stratum/<name>` es un descenso de capa y
/// cualquier otra cosa es un componente común. `subsystems/bilinker>impl` es una
/// de cada una, y esa mezcla es la forma de este proyecto, no un caso raro.
fn filesystem_to_stratum_tokens(rel: &Path) -> Result<stratum::StratumPath> {
    use stratum::PathToken;

    let components: Vec<Component> = rel.components().collect();
    let mut tokens = Vec::new();
    let mut plain: Vec<&std::ffi::OsStr> = Vec::new();

    let flush = |plain: &mut Vec<&std::ffi::OsStr>, tokens: &mut stratum::StratumPath| {
        if !plain.is_empty() {
            tokens.push(PathToken::Simple(plain.iter().collect()));
            plain.clear();
        }
    };

    let mut i = 0;
    while i < components.len() {
        if let (Some(Component::Normal(a)), Some(Component::Normal(b))) =
            (components.get(i), components.get(i + 1))
        {
            if *a == std::ffi::OsStr::new(".stratum") {
                flush(&mut plain, &mut tokens);
                let name = b.to_str().ok_or_else(|| anyhow::anyhow!("non-UTF8 layer name"))?;
                tokens.push(PathToken::Down(name.to_string()));
                i += 2;
                continue;
            }
        }
        match components[i] {
            Component::Normal(n) => plain.push(n),
            other => bail!("componente inesperado en un path de capa: {other:?}"),
        }
        i += 1;
    }
    flush(&mut plain, &mut tokens);

    if tokens.is_empty() {
        bail!("empty stratum path for {}", rel.display());
    }
    Ok(tokens)
}


fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ep(raw: &str) -> LinkEndpoint { raw.parse().unwrap() }

    fn inverse(from: &str, to: &str) -> String {
        layer_endpoint(Path::new(from), Path::new(to)).unwrap().to_string()
    }

    /// El endpoint inverso se cuenta en capas, no en componentes de path.
    ///
    /// Con `subsystems/bilinker>impl` en el medio, un diff de paths ve cuatro `..`
    /// y escribe `<<`. Pero sólo se cruzó una capa: `subsystems/bilinker` son
    /// directorios comunes, y `<` los atraviesa solo al buscar la raíz verdadera.
    #[test]
    fn plain_directories_do_not_count_as_layer_levels() {
        assert_eq!(inverse("subsystems/bilinker/.stratum/impl", ""), "path <");
        assert_eq!(inverse("", "subsystems/bilinker/.stratum/impl"),
                   "path subsystems/bilinker>impl");
    }

    /// El caso sin directorios comunes sigue igual.
    #[test]
    fn a_layer_directly_below_inverts_to_one_up() {
        assert_eq!(inverse(".stratum/impl", ""), "path <");
        assert_eq!(inverse("", ".stratum/impl"), "path >impl");
    }

    /// Dos niveles de capa sí son dos `<`.
    #[test]
    fn nested_layers_count_once_each() {
        assert_eq!(inverse("a/.stratum/mid/.stratum/impl", ""), "path <<");
    }

    /// Una cadena entre dos capas: un bilink en cada una, con el mismo uuid.
    #[test]
    fn a_two_layer_chain_writes_one_file_per_layer() {
        let d = tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".stratum/impl")).unwrap();

        let r = chain_new(d.path(),
            &[(PathBuf::from("."), ep("capture aaa")),
              (PathBuf::from(".stratum/impl"), ep("capture bbb"))],
            &[], &Declaration::default()).unwrap();

        assert_eq!(r.files.len(), 2);
        for f in &r.files { assert!(f.exists(), "no se escribió {}", f.display()); }

        // El tip de la capa raíz apunta a su capture y a la capa vecina.
        let spec = BiLink::load(&r.files[0]).unwrap();
        assert_eq!(spec.endpoint.zero.link.to_string(), "capture aaa");
        assert_eq!(spec.endpoint.one.link.prefix(), "path");
    }

    /// Los dos endpoints en la misma capa: un solo archivo, sin traversal.
    #[test]
    fn a_direct_link_writes_a_single_file() {
        let d = tempdir().unwrap();
        let r = chain_new(d.path(),
            &[(PathBuf::from("."), ep("capture aaa")),
              (PathBuf::from("."), ep("capture bbb"))],
            &[], &Declaration::default()).unwrap();
        assert_eq!(r.files.len(), 1);
    }

    /// Una cadena nace sin nada aceptado: su ausencia *es* PENDING.
    #[test]
    fn a_fresh_chain_has_nothing_accepted() {
        let d = tempdir().unwrap();
        let r = chain_new(d.path(),
            &[(PathBuf::from("."), ep("capture aaa")),
              (PathBuf::from("."), ep("capture bbb"))],
            &[], &Declaration::default()).unwrap();
        let bl = BiLink::load(&r.files[0]).unwrap();
        assert!(bl.endpoint.zero.accepted.is_empty());
        assert!(bl.endpoint.one.accepted.is_empty());
    }
}
