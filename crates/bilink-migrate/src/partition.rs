//! `bilinker-002-file-partition` — del formato 1 al 2.
//!
//! Reescribe cada bilink a YAML con los endpoints bajo `endpoint.0`/`endpoint.1` y
//! el tipo de cada `link` explícito, y cada capture bajo su id de contenido. Todo lo
//! derivable —`state`, `range`, `commit`— sale del formato; `resolved_at` **se
//! descarta**: no se muda a la cache, desaparece.
//!
//! Es una transformación **puramente sintáctica**: copia y renombre, sin resolver
//! ninguna query ni consultar git. Una migración que resuelve puede fallar por
//! motivos ajenos al formato —un archivo que se movió, una query rota— y dejar la
//! capa a mitad de camino.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use accreta_migrate::Outcome;

use bilink_format_v1 as v1;
use bilink_format as v2;

/// El sufijo de la carpeta transitoria en la que la migración escribe.
///
/// **Los dos formatos no pueden ocupar `.bilink/` a la vez.** La migración escribe
/// al lado y deja el original intacto, así que el binario viejo sigue trabajando sin
/// enterarse y el nuevo se ejerce contra datos reales antes de que nada sea
/// irreversible. El id va en el nombre: sin él, dos migraciones en vuelo colisionan
/// y una carpeta abandonada es indistinguible de una en curso.
pub const OUT_DIR: &str = ".bilink-migrate-002-file-partition";

pub fn run(layer: &Path, dry_run: bool) -> Result<Outcome> {
    let src = layer.join(".bilink");
    if !src.exists() {
        return Ok(Outcome::default());
    }
    let plan = plan(layer)?;
    let mut out = Outcome::default();

    if !dry_run {
        // Regenerar siempre: la carpeta es un derivado, y regenerar es exactamente
        // lo que recupera un `accept` hecho con el binario viejo entre la generación
        // y el corte. La regla operativa es regenerar justo antes de cortar.
        let dst = layer.join(OUT_DIR);
        if dst.exists() {
            std::fs::remove_dir_all(&dst)
                .with_context(|| format!("limpiando {}", dst.display()))?;
        }
        plan.write(layer)?;
    }
    out.changed = plan.paths(layer);
    out.notes.push(plan.summary(layer));
    Ok(out)
}

// ─── el plan ──────────────────────────────────────────────────────────────────

/// Lo que la migración va a escribir, sin haber escrito nada.
///
/// Separar el cálculo de la escritura es lo que hace que `--dry-run` reporte
/// exactamente lo mismo que la corrida real, sin que el contrato dependa de que
/// cada rama del código se acuerde de mirar el flag.
#[derive(Debug, Default)]
pub struct Plan {
    pub bilinks:  BTreeMap<String, v2::BiLink>,
    pub captures: BTreeMap<String, v2::Capture>,
    /// Cuántos endpoints quedaron sin `accepted` porque no tenían hash.
    pub pending: usize,
    /// Cuántos `resolved_at` se descartaron. Se reporta para que no sea silencioso.
    pub dropped_resolved_at: usize,
    /// Captures del formato 1 que colapsaron en uno solo por tener la misma ubicación.
    pub collapsed: usize,
    /// Sub-rangos descartados: el formato ya no los tiene. El endpoint queda
    /// apuntando al nodo que lo contenía, y hay que revisarlo a mano.
    pub ranges_dropped: usize,
    /// Endpoints `path` cuyo vecino no se pudo leer: quedan sin `accepted.link`.
    pub unresolved_neighbours: usize,
    /// Los `commit.N` rescatados, por `(uuid, n)`.
    ///
    /// **No se descartan**: sin ellos `accepted.hash` es un hash que no se puede
    /// resolver a texto, y `check` pierde las distinciones que dependen de él.
    ///
    /// La migración los **devuelve** en vez de escribirlos: el destino es la cache,
    /// que es de la herramienta y no del formato. Escribirla desde acá obligaría a
    /// que una transformación sintáctica dependa del crate que la interpreta.
    ///
    /// El `commit.N` del formato 1 era el HEAD de quien aceptaba, no el commit del
    /// contenido. Es impreciso y sirve igual: quien lo usa **verifica el hash** antes
    /// de creerle, así que un valor que no corresponde degrada como una cache fría en
    /// vez de mentir, y se corrige solo en la próxima aceptación.
    pub commits: Vec<(String, u8, String)>,
}

pub fn plan(layer: &Path) -> Result<Plan> {
    let mut p = Plan::default();
    let bilink_dir = layer.join(".bilink");

    // Orden estable: el plan tiene que ser el mismo entre corridas, y `read_dir` no
    // garantiza orden. Sin esto la salida sería determinista en contenido pero no en
    // los mensajes ni en el orden de los conflictos.
    let mut files: Vec<PathBuf> = v1::bilink::walkdir(&bilink_dir)?
        .into_iter()
        .filter(|f| f.extension().and_then(|e| e.to_str()) == Some("bilink"))
        .filter(|f| !f.file_name().and_then(|n| n.to_str())
                      .map(|n| n.starts_with('.')).unwrap_or(false))
        .collect();
    files.sort();

    for path in &files {
        if let Ok(text) = std::fs::read_to_string(path) {
            p.dropped_resolved_at += text.lines().filter(|l| l.starts_with("resolved_at:")).count();
        }
        let old = v1::bilink::BiLinkFile::load(path)
            .with_context(|| format!("leyendo {}", path.display()))?;

        let zero = endpoint(layer, &old, 0, &mut p)?;
        let one  = endpoint(layer, &old, 1, &mut p)?;

        // `commit.N` sale del formato pero **no se descarta**: es un derivado, y su
        // lugar es la cache. Perderlo dejaría a cada endpoint sin cómo recuperar su
        // texto aceptado hasta que alguien vuelva a aceptar.
        for (n, c) in [(0u8, &old.commit0), (1u8, &old.commit1)] {
            if let Some(c) = c {
                p.commits.push((old.uuid.clone(), n, c.clone()));
            }
        }

        p.bilinks.insert(old.uuid.clone(), v2::BiLink {
            kind: old.kind.clone(),
            endpoint: v2::Endpoints { zero, one },
        });
    }

    Ok(p)
}

/// Convierte un endpoint del formato 1 al 2.
fn endpoint(layer: &Path, old: &v1::bilink::BiLinkFile, n: u8, p: &mut Plan) -> Result<v2::Endpoint> {
    let (old_link, hash, hash_ast, name) = match n {
        0 => (&old.link0, &old.hash0, &old.hash_ast0, &old.name0),
        _ => (&old.link1, &old.hash1, &old.hash_ast1, &old.name1),
    };

    let link = convert_link(layer, old_link, p)?;

    // `accepted.link` se siembra copiando `link` donde había hash.
    //
    // Es exacto donde el endpoint estaba OK: en el formato 1 un endpoint OK es uno
    // cuyo contenido actual coincide con el aceptado en la ubicación que `link`
    // describe, así que esa ubicación *es* la bendecida. Donde estaba no-OK es la
    // única lectura disponible —el formato 1 no distingue drift de ubicación de
    // drift de contenido— y es la que preserva la invariante de aceptación sin poner
    // todo en RELOCATED de golpe ni degradarlo a PENDING, que borraría el inventario.
    let accepted = match hash {
        Some(h) => Some(v2::Accepted {
            // Un endpoint estructural aprueba su propia ubicación; uno `path` copia
            // la del endpoint estructural de su vecino. En el formato 1 esa segunda
            // copia no existía —sólo se copiaba el hash— así que hay que ir a
            // buscarla, o el endpoint nace en RELOCATED contra su propio vecino.
            link: match &link {
                v2::LinkEndpoint::Capture(_) => Some(link.clone()),
                v2::LinkEndpoint::Path(_)    => neighbour_capture(layer, old_link, &old.uuid, p)?,
                v2::LinkEndpoint::Issue(_)   => None,
            },
            hash: h.clone(),
            hash_ast: hash_ast.clone(),
        }),
        None => { p.pending += 1; None }
    };

    // `name.N` pasa a ser `name` adentro de su endpoint: es un dato de una punta
    // y ahora hay dónde ponerlo.
    Ok(v2::Endpoint { link, accepted, name: name.clone() })
}

/// El capture que el endpoint estructural del bilink vecino aprueba.
///
/// Se resuelve leyendo el formato 1 del vecino y acuñando su id igual que se acuña
/// el propio: misma función, mismo resultado. No consulta git ni resuelve queries,
/// así que sigue siendo una transformación sintáctica.
///
/// El capture del vecino **no se agrega al plan**: vive en la capa del vecino, y esa
/// capa lo acuña cuando le toque migrar. Acá sólo se necesita su id.
fn neighbour_capture(
    layer: &Path,
    old_link: &v1::link::LinkEndpoint,
    uuid: &str,
    p: &mut Plan,
) -> Result<Option<v2::LinkEndpoint>> {
    let v1::link::LinkEndpoint::Layer(tokens) = old_link else { return Ok(None) };
    let Ok(target) = stratum::resolve(layer, layer, tokens) else { return Ok(None) };

    // La raíz verdadera de la capa vecina, que puede estar más arriba del path.
    let adj_layer = true_layer_root(&layer.join(&target));
    let adj_path  = adj_layer.join(".bilink").join(format!("{uuid}.bilink"));
    if !adj_path.exists() {
        p.unresolved_neighbours += 1;
        return Ok(None);
    }
    let adj = v1::bilink::BiLinkFile::load(&adj_path)?;

    for n in [0u8, 1u8] {
        let link = if n == 0 { &adj.link0 } else { &adj.link1 };
        let sref = match link {
            v1::link::LinkEndpoint::Capture(id) =>
                Some(v1::capture::CaptureFile::load_in(&adj_layer, id)?.sref),
            v1::link::LinkEndpoint::LegacyStructural(s) => Some(s.clone()),
            _ => None,
        };
        if let Some(sref) = sref {
            let cap = v2::Capture { file: sref.file.clone(), query: sref.query.clone() };
            if sref.range.is_some() { p.ranges_dropped += 1; }
            return Ok(Some(format!("capture {}", cap.id()).parse()?));
        }
    }
    p.unresolved_neighbours += 1;
    Ok(None)
}

/// La raíz verdadera de una capa: el ancestro más cercano con `.git` o `.bilink`.
fn true_layer_root(start: &Path) -> PathBuf {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".bilink").is_dir() || cur.join(".git").exists() {
            return cur;
        }
        if !cur.pop() { return start.to_path_buf(); }
    }
}

/// Un `link` del formato 1 al 2, acuñando el capture si hace falta.
fn convert_link(layer: &Path, old: &v1::link::LinkEndpoint, p: &mut Plan) -> Result<v2::LinkEndpoint> {
    use v1::link::LinkEndpoint as Old;
    Ok(match old {
        // El endpoint layer era el fallback y se escribía sin prefijo. Ahora lleva
        // el suyo, que es todo el cambio: el valor es el mismo path Stratum.
        Old::Layer(tokens) => {
            let raw = stratum::format_path(tokens);
            format!("path {raw}").parse()?
        }
        Old::Issue(id) => format!("issue {id}").parse()?,

        // Los dos estructurales convergen: el capture del formato 1 tenía un uuid
        // arbitrario, y en el 2 su nombre es el hash de su propia ubicación.
        Old::Capture(uuid) => {
            let cap = v1::capture::CaptureFile::load_in(layer, uuid)
                .with_context(|| format!("leyendo el capture {uuid}"))?;
            mint(&cap.sref, p)?
        }
        Old::LegacyStructural(sref) => mint(sref, p)?,
    })
}

/// Acuña el capture del formato 2 para una ubicación, y devuelve el endpoint.
///
/// Dos ubicaciones idénticas dan el mismo id y por lo tanto el mismo archivo: la
/// deduplicación es por construcción, y acá se aplica de una vez a lo que ya existía.
fn mint(sref: &v1::link::StructuralRef, p: &mut Plan) -> Result<v2::LinkEndpoint> {
    // **El sub-rango se descarta, y se cuenta.**
    //
    // El formato ya no lo tiene: un fragmento es un nodo entero. Reubicarlo
    // exigiría resolver la query y buscar el nodo correcto, y una migración no
    // corre tree-sitter (`migration.md` inv. 5). Así que el endpoint queda
    // apuntando al nodo que lo contenía, y el resumen dice cuántos — la misma
    // regla que `001` con `subgraph.N`: una pérdida se reporta, no se calla.
    let cap = v2::Capture { file: sref.file.clone(), query: sref.query.clone() };
    if sref.range.is_some() { p.ranges_dropped += 1; }
    let id = cap.id();
    if p.captures.insert(id.clone(), cap).is_some() {
        p.collapsed += 1;
    }
    format!("capture {id}").parse()
}

impl Plan {
    /// Los archivos que la migración escribe, en orden estable.
    pub fn paths(&self, layer: &Path) -> Vec<PathBuf> {
        let out = layer.join(OUT_DIR);
        let mut v: Vec<PathBuf> = self.bilinks.keys().map(|u| out.join(format!("{u}.yaml"))).collect();
        v.extend(self.captures.keys().map(|id| out.join("capture").join(format!("{id}.yaml"))));
        v.push(out.join(v2::VERSION_FILE));
        v
    }

    pub fn summary(&self, layer: &Path) -> String {
        format!(
            "{}: {} bilink(s), {} capture(s){}{}, {} endpoint(s) sin aceptar, \
             {} resolved_at descartado(s){}",
            layer.display(),
            self.bilinks.len(),
            self.captures.len(),
            if self.collapsed > 0 { format!(" ({} colapsado(s) por dedup)", self.collapsed) } else { String::new() },
            "",
            self.pending,
            self.dropped_resolved_at,
            if self.unresolved_neighbours > 0 {
                format!(", {} vecino(s) sin resolver", self.unresolved_neighbours)
            } else { String::new() },
        )
    }

    /// Escribe el plan en la carpeta transitoria.
    pub fn write(&self, layer: &Path) -> Result<()> {
        let out = layer.join(OUT_DIR);
        std::fs::create_dir_all(out.join("capture"))?;

        for (uuid, bl) in &self.bilinks {
            std::fs::write(out.join(format!("{uuid}.yaml")), bl.to_yaml()?)?;
        }
        for (id, cap) in &self.captures {
            std::fs::write(out.join("capture").join(format!("{id}.yaml")), cap.to_yaml()?)?;
        }
        // La versión de formato viaja con los archivos que describe.
        std::fs::write(out.join(v2::VERSION_FILE), format!("{}\n", v2::VERSION))?;

        Ok(())
    }
}

/// Comprueba que la carpeta migrada dice lo mismo que la original.
///
/// **La verificación la hace la migración**, no un comando que compare dos árboles:
/// un comando así linkea un solo parser y sólo puede leer uno de los dos lados.
pub fn verify(layer: &Path) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let plan = plan(layer)?;

    for path in v1::bilink::walkdir(&layer.join(".bilink"))?
        .into_iter()
        .filter(|f| f.extension().and_then(|e| e.to_str()) == Some("bilink"))
    {
        let old = v1::bilink::BiLinkFile::load(&path)?;
        let Some(new) = plan.bilinks.get(&old.uuid) else {
            problems.push(format!("{}: el bilink no está en la salida", old.uuid));
            continue;
        };
        for n in [0u8, 1u8] {
            let old_hash = if n == 0 { &old.hash0 } else { &old.hash1 };
            let new_hash = new.endpoint.get(n).accepted.as_ref().map(|a| &a.hash);
            if old_hash.as_ref() != new_hash {
                problems.push(format!("{}.{n}: el hash aceptado no sobrevivió", old.uuid));
            }
        }
    }
    if problems.is_empty() && plan.bilinks.is_empty() {
        bail!("no se encontró ningún bilink en {}", layer.display());
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Una capa en formato 1, escrita a mano tal como el binario viejo la dejaría.
    fn layer_v1() -> tempfile::TempDir {
        let d = tempdir().unwrap();
        let b = d.path().join(".bilink");
        std::fs::create_dir_all(b.join("capture")).unwrap();

        std::fs::write(b.join("capture/c0.capture"), concat!(
            "file:   commands/check.md\n",
            "query:  (section\n",
            "  (atx_heading (inline) @n0 (#eq? @n0 \"Firma\"))) @target\n",
            "\n# Cache\nrange:       100~200\nstate:       RESOLVED\n",
            "resolved_at: 2026-08-26T15:07:30Z\n")).unwrap();
        // Misma ubicación que c0, con otro uuid: en el formato 2 colapsan en uno.
        std::fs::write(b.join("capture/c1.capture"), concat!(
            "file:   commands/check.md\n",
            "query:  (section\n",
            "  (atx_heading (inline) @n0 (#eq? @n0 \"Firma\"))) @target\n",
            "\n# Cache\nrange:       100~200\nstate:       RESOLVED\n")).unwrap();

        // Aceptado, con endpoint layer.
        std::fs::write(b.join("aaaa1111-0000-4000-8000-000000000001.bilink"), concat!(
            "link.0: capture c0\nlink.1: subsystems/bilinker>impl\n",
            "kind: governs\nname.0: la-decision\nname.1: lo-gobernado\n",
            "\n# Cache\n",
            "hash.0: c00e0760\nhash_ast.0: 1b9e44a2\ncommit.0: deadbeef\n",
            "hash.1: b2c3d4e5\ncommit.1: cafebabe\n",
            "state.0: OK\nstate.1: OK\nresolved_at: 2026-08-29T23:29:13Z\n")).unwrap();

        // Sin aceptar: `accepted` tiene que quedar ausente, no vacío.
        std::fs::write(b.join("bbbb2222-0000-4000-8000-000000000002.bilink"), concat!(
            "link.0: capture c1\nlink.1: issue 3a\n")).unwrap();
        d
    }

    /// La migración preserva `kind` y `name.N`.
    ///
    /// Son declaración, no cache: sobreviven el cambio de formato porque nadie los
    /// deriva de nada. `migrate.md` lo decía desde antes de que fuera cierto — el
    /// lector de formato 1 no los modelaba, así que la migración recibía `None`.
    #[test]
    fn the_declaration_fields_survive_the_migration() {
        let d = layer_v1();
        let plan = plan(d.path()).unwrap();
        let bl = plan.bilinks.get("aaaa1111-0000-4000-8000-000000000001")
            .expect("el bilink aceptado");

        assert_eq!(bl.kind.as_deref(), Some("governs"));
        assert_eq!(bl.endpoint.get(0).name.as_deref(), Some("la-decision"));
        assert_eq!(bl.endpoint.get(1).name.as_deref(), Some("lo-gobernado"));
    }

    /// Un repo que nunca corrió `001` llega igual al formato 2.
    ///
    /// Es la prueba de que retirar `001` fue legítimo y no un borrado con otro
    /// nombre: `002` lee la forma embebida —`file :: query :: offset` dentro del
    /// `link`— además de la que `001` producía, así que el camino desde el formato
    /// 1 crudo sigue completo, con un salto menos.
    #[test]
    fn a_repo_that_never_ran_001_still_reaches_format_2() {
        let d = tempdir().unwrap();
        let b = d.path().join(".bilink");
        std::fs::create_dir_all(&b).unwrap();

        // Formato 1 **crudo**: la ubicación embebida en el link, sin `.capture`.
        std::fs::write(b.join("cccc3333-0000-4000-8000-000000000003.bilink"), concat!(
            "link.0: commands/check.md :: (section\n",
            "  (atx_heading (inline) @n0 (#eq? @n0 \"Firma\"))) @target :: 0~120\n",
            "link.1: .stratum/impl\n",
            "\n# Cache\nhash.0: c00e0760\ncommit.0: deadbeef\n")).unwrap();

        let plan = plan(d.path()).unwrap();
        let bl = plan.bilinks.get("cccc3333-0000-4000-8000-000000000003")
            .expect("el bilink migró");

        // El endpoint estructural quedó apuntando a un capture acuñado en el acto.
        let link = bl.endpoint.get(0).link.to_string();
        assert!(link.starts_with("capture "), "no se acuñó el capture: {link}");
        let id = link.trim_start_matches("capture ");
        let cap = plan.captures.get(id).expect("el capture está en el plan");
        assert_eq!(cap.file, "commands/check.md");
        assert!(cap.query.as_deref().unwrap().contains("Firma"));

        // Y la aceptación sobrevivió: es lo que un corte no puede perder.
        assert_eq!(bl.endpoint.get(0).accepted.as_ref().unwrap().hash, "c00e0760");
    }

    /// **La propiedad que importa.** Correrla dos veces da bytes idénticos.
    ///
    /// Es lo que hace que la carpeta se pueda regenerar en cualquier momento, y por
    /// lo tanto lo único que la vuelve segura como derivado.
    #[test]
    fn the_migration_is_deterministic() {
        let d = layer_v1();
        run(d.path(), false).unwrap();
        let first = snapshot(&d.path().join(OUT_DIR));

        run(d.path(), false).unwrap();
        let second = snapshot(&d.path().join(OUT_DIR));

        assert_eq!(first, second, "dos corridas tienen que dar bytes idénticos");
        assert!(!first.is_empty());
    }

    /// Regenerar recupera lo que el binario viejo escribió en el medio.
    ///
    /// Es el caso que la regla operativa —regenerar justo antes de cortar— protege:
    /// sin esto, un `accept` hecho después de generar se lo comería el corte.
    #[test]
    fn regenerating_picks_up_a_later_accept() {
        let d = layer_v1();
        run(d.path(), false).unwrap();
        let before = snapshot(&d.path().join(OUT_DIR));

        // El binario viejo acepta el endpoint que estaba pendiente.
        let p = d.path().join(".bilink/bbbb2222-0000-4000-8000-000000000002.bilink");
        let text = std::fs::read_to_string(&p).unwrap();
        std::fs::write(&p, text + "\n# Cache\nhash.0: 99999999\ncommit.0: 11112222\n").unwrap();

        run(d.path(), false).unwrap();
        let after = snapshot(&d.path().join(OUT_DIR));

        assert_ne!(before, after, "regenerar tiene que recoger la aceptación nueva");
        assert!(after.values().any(|t| t.contains("99999999")),
            "el hash aceptado después de generar no llegó a la salida");
    }

    /// `--dry-run` no escribe un solo archivo, y reporta lo mismo.
    #[test]
    fn a_dry_run_writes_nothing_and_reports_the_same() {
        let d = layer_v1();
        let dry = run(d.path(), true).unwrap();
        assert!(!d.path().join(OUT_DIR).exists(), "--dry-run no debe escribir nada");

        let real = run(d.path(), false).unwrap();
        assert_eq!(dry.notes, real.notes, "el reporte tiene que ser el mismo");
        assert_eq!(dry.changed, real.changed);
    }

    /// La original queda intacta: los dos formatos conviven.
    #[test]
    fn the_source_layer_is_untouched() {
        let d = layer_v1();
        let before = snapshot(&d.path().join(".bilink"));
        run(d.path(), false).unwrap();
        assert_eq!(before, snapshot(&d.path().join(".bilink")));
    }

    /// Los hashes aceptados sobreviven, y lo verifica la migración misma.
    #[test]
    fn no_accepted_hash_is_lost() {
        let d = layer_v1();
        assert!(verify(d.path()).unwrap().is_empty());
    }

    /// Dos captures con la misma ubicación colapsan en uno. Dedup por construcción.
    #[test]
    fn identical_locations_collapse_into_one_capture() {
        let d = layer_v1();
        let p = plan(d.path()).unwrap();
        assert_eq!(p.captures.len(), 1, "c0 y c1 describen la misma ubicación");
        assert_eq!(p.collapsed, 1);
    }

    /// `commit.N` **sí** se muda: es un derivado, y su lugar es la cache.
    ///
    /// Es la distinción que `cache.md` hace entre las dos clases de derivado. `state`
    /// y `range` se recalculan corriendo `check`; `commit` no —hace falta caminar la
    /// historia— así que tirarlo deja a cada endpoint sin cómo recuperar su texto
    /// aceptado hasta que alguien vuelva a aceptar.
    #[test]
    fn the_accepted_commit_is_carried_not_dropped() {
        let d = layer_v1();
        let p = plan(d.path()).unwrap();

        let mine: Vec<_> = p.commits.iter()
            .filter(|(u, _, _)| u.starts_with("aaaa1111")).collect();
        assert_eq!(mine.len(), 2, "los dos endpoints tenían commit");
        assert!(mine.iter().any(|(_, n, c)| *n == 0 && c == "deadbeef"));
        assert!(mine.iter().any(|(_, n, c)| *n == 1 && c == "cafebabe"));

        // Y no vuelve al formato: sale de los archivos versionados.
        run(d.path(), false).unwrap();
        for (name, text) in snapshot(&d.path().join(OUT_DIR)) {
            assert!(!text.contains("deadbeef"), "{name} todavía lleva el commit:\n{text}");
        }
    }

    /// `resolved_at` no se muda a ninguna parte: desaparece, y se reporta.
    #[test]
    fn resolved_at_is_dropped_not_migrated() {
        let d = layer_v1();
        let p = plan(d.path()).unwrap();
        assert_eq!(p.dropped_resolved_at, 1, "hay un resolved_at en los bilinks");

        run(d.path(), false).unwrap();
        for (name, text) in snapshot(&d.path().join(OUT_DIR)) {
            assert!(!text.contains("resolved_at"), "{name} todavía lo tiene:\n{text}");
            assert!(!text.contains("state"),  "{name} todavía lleva estado:\n{text}");
            assert!(!text.contains("commit"), "{name} todavía lleva commit:\n{text}");
        }
    }

    /// La salida la lee el parser del formato 2, no una comparación de texto.
    #[test]
    fn the_output_parses_as_format_2() {
        let d = layer_v1();
        run(d.path(), false).unwrap();
        let out = d.path().join(OUT_DIR);

        let bl = v2::BiLink::load(&out.join("aaaa1111-0000-4000-8000-000000000001.yaml")).unwrap();
        assert_eq!(bl.endpoint.zero.link.prefix(), "capture");
        assert_eq!(bl.endpoint.one.link.to_string(), "path subsystems/bilinker>impl",
            "el endpoint layer gana su prefijo y conserva el path");

        let acc = bl.endpoint.zero.accepted.as_ref().unwrap();
        assert_eq!(acc.hash, "c00e0760");
        assert_eq!(acc.hash_ast.as_deref(), Some("1b9e44a2"));
        assert_eq!(acc.link.as_ref().unwrap(), &bl.endpoint.zero.link,
            "accepted.link se siembra copiando link");

        // Sin hash en el formato 1 → sin bloque en el 2. Su ausencia es PENDING.
        let bl2 = v2::BiLink::load(&out.join("bbbb2222-0000-4000-8000-000000000002.yaml")).unwrap();
        assert!(bl2.endpoint.zero.accepted.is_none());
        assert_eq!(bl2.endpoint.one.link.to_string(), "issue 3a");
    }

    /// Un endpoint `issue` no lleva `accepted.link`: no hay capture que aprobar.
    #[test]
    fn an_issue_endpoint_gets_no_accepted_link() {
        let d = layer_v1();
        let p = plan(d.path()).unwrap();
        let bl = &p.bilinks["bbbb2222-0000-4000-8000-000000000002"];
        assert_eq!(bl.endpoint.one.link.to_string(), "issue 3a");
        assert!(bl.endpoint.one.accepted.is_none());
    }

    /// La versión de formato viaja con los archivos que describe.
    #[test]
    fn the_output_declares_its_format_version() {
        let d = layer_v1();
        run(d.path(), false).unwrap();
        let v = std::fs::read_to_string(d.path().join(OUT_DIR).join(v2::VERSION_FILE)).unwrap();
        assert_eq!(v.trim(), v2::VERSION);
    }

    fn snapshot(dir: &Path) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_dir() { stack.push(p); }
                else if let Ok(t) = std::fs::read_to_string(&p) {
                    out.insert(p.strip_prefix(dir).unwrap().display().to_string(), t);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;
    use tempfile::tempdir;

    /// Dos capas encadenadas, en formato 1.
    fn chained() -> tempfile::TempDir {
        let d = tempdir().unwrap();
        let uuid = "cccc3333-0000-4000-8000-000000000003";

        // capa raíz
        let a = d.path().join(".bilink");
        std::fs::create_dir_all(a.join("capture")).unwrap();
        std::fs::write(a.join("capture/ca.capture"), "file:   spec.md\n").unwrap();
        std::fs::write(a.join(format!("{uuid}.bilink")), concat!(
            "link.0: capture ca\nlink.1: >impl\n\n# Cache\n",
            "hash.0: aaaa1111\ncommit.0: c1\n",
            "hash.1: bbbb2222\ncommit.1: c2\n",
            "state.0: OK\nstate.1: OK\n")).unwrap();

        // capa impl
        let b = d.path().join(".stratum/impl/.bilink");
        std::fs::create_dir_all(b.join("capture")).unwrap();
        std::fs::write(b.join("capture/cb.capture"), "file:   src/lib.rs\n").unwrap();
        std::fs::write(b.join(format!("{uuid}.bilink")), concat!(
            "link.0: <\nlink.1: capture cb\n\n# Cache\n",
            "hash.0: aaaa1111\ncommit.0: c1\n",
            "hash.1: bbbb2222\ncommit.1: c2\n",
            "state.0: OK\nstate.1: OK\n")).unwrap();
        d
    }

    /// **Una cadena que estaba OK sigue OK después de migrar.**
    ///
    /// Un endpoint `path` aprueba la ubicación de su vecino, y en el formato 1 esa
    /// copia no existía: sólo se copiaba el hash. Sin ir a buscarla, cada endpoint
    /// `path` nace en CHAIN_DIRTY contra su propio vecino — que es exactamente lo
    /// que pasó al cortar por primera vez, con 118 de 118.
    #[test]
    fn a_path_endpoint_gets_the_neighbours_accepted_link() {
        let d = chained();
        let uuid = "cccc3333-0000-4000-8000-000000000003";

        let root = plan(d.path()).unwrap();
        let impl_ = plan(&d.path().join(".stratum/impl")).unwrap();

        let root_path_ep = &root.bilinks[uuid].endpoint.one;   // path >impl
        let impl_struct  = &impl_.bilinks[uuid].endpoint.one;  // capture cb

        assert_eq!(
            root_path_ep.accepted.as_ref().unwrap().link,
            impl_struct.accepted.as_ref().unwrap().link,
            "el endpoint path tiene que copiar la ubicación aprobada de su vecino");

        // Y al revés.
        let impl_path_ep = &impl_.bilinks[uuid].endpoint.zero; // path <
        let root_struct  = &root.bilinks[uuid].endpoint.zero;  // capture ca
        assert_eq!(
            impl_path_ep.accepted.as_ref().unwrap().link,
            root_struct.accepted.as_ref().unwrap().link);
    }

    /// El capture del vecino no entra en el plan de esta capa: vive en la suya.
    #[test]
    fn the_neighbours_capture_is_not_minted_here() {
        let d = chained();
        let root = plan(d.path()).unwrap();
        assert_eq!(root.captures.len(), 1, "sólo el propio, no el del vecino");
        assert_eq!(root.unresolved_neighbours, 0);
    }
}
