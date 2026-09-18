//! `bilinker-004-dimensions` — un capture compuesto pasa a ancla y dimensiones.
//!
//! Un capture escrito cuando la query componía el fragmento lleva varios `@target`,
//! y su `hash` aceptado es el de la concatenación. Acá se reescribe en la forma de
//! hoy: el capture del nodo, y las partes como dimensiones del endpoint. Los
//! archivos son `4.1.0` antes y después.
//!
//! # No acepta: parte una aceptación que ya existe
//!
//! Por eso migra **todo o nada**. Cada endpoint compuesto de la capa tiene que estar
//! `OK` —una sola aceptación, en la ubicación aprobada, con el contenido aprobado— y
//! las partes de hoy tienen que reproducir el fragmento byte a byte. Ahí el hash de
//! cada parte es un pedazo de lo que alguien aprobó. Si uno solo no cumple, no se
//! escribe nada y se nombra cuál.
//!
//! # La única que resuelve queries
//!
//! Las demás son funciones de los archivos de `.bilink/` y nada más. Ésta parsea los
//! archivos de la capa, porque el ancla nueva y las partes salen de la gramática; y
//! sigue sin consultar git, que es lo que haría depender el resultado de qué
//! historia alcanza cada clon.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use accreta_migrate::Outcome;
use bilink_format::{BiLink, Capture, LinkEndpoint};

pub const OUT_DIR: &str = ".bilink-migrate-004-dimensions";
pub const BACKUP_DIR: &str = ".bilink-formato-4-compuesto";

#[derive(Default)]
pub struct Plan {
    /// Los bilinks reescritos, por nombre de archivo.
    pub files: BTreeMap<String, String>,
    /// Los captures de los nodos, sin repetir.
    pub captures: BTreeMap<String, Capture>,
    /// Los endpoints partidos, como `<uuid>.<N>`.
    pub migrated: Vec<String>,
    /// Los que impiden migrar, con lo que les falta.
    pub refused: Vec<(String, String)>,
}

/// Qué haría la migración en `layer`, sin escribir nada.
pub fn plan(layer: &Path) -> Result<Plan> {
    let mut p = Plan::default();
    let dir = layer.join(".bilink");
    let major = bilink_format::read_version(layer)
        .and_then(|v| v.split('.').next()?.parse::<u32>().ok());
    if !dir.exists() || major != Some(4) {
        return Ok(p);
    }

    let mut files: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|f| f.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .filter(|f| !f.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')))
        .collect();
    files.sort();

    for path in &files {
        let mut bl = BiLink::load(path)?;
        let uuid = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string();
        let mut changed = false;
        for n in [0u8, 1u8] {
            let Some(id) = bl.endpoint.get(n).link.capture_id().map(str::to_string) else { continue };
            let cap = Capture::load_in(layer, &id)?;
            if !cap.query.as_deref().is_some_and(bilinker::composed::is_composed) { continue; }

            let at = format!("{}.{n}", &uuid[..8.min(uuid.len())]);
            let other = &bl.endpoint.get(1 - n).link;
            match split_endpoint(layer, &bl, n, &cap, other) {
                Err(e) => p.refused.push((at, e.to_string())),
                Ok(s) => {
                    let new_link = LinkEndpoint::Capture(s.capture.id());
                    let e = bl.endpoint.get_mut(n);
                    e.link = new_link.clone();
                    e.dimensions = s.dimensions;
                    let a = &mut e.accepted[0];
                    a.link = Some(new_link);
                    a.hash = s.hash;
                    a.hash_ast = s.hash_ast;
                    a.dimensions = s.accepted;
                    p.captures.insert(s.capture.id(), s.capture);
                    p.migrated.push(at);
                    changed = true;
                }
            }
        }
        if changed {
            let name = path.file_name().and_then(|n| n.to_str()).context("nombre de archivo")?;
            p.files.insert(name.to_string(), bl.to_yaml()?);
        }
    }
    Ok(p)
}

/// Las condiciones de un endpoint compuesto, y su aceptación partida.
fn split_endpoint(
    layer: &Path,
    bl: &BiLink,
    n: u8,
    cap: &Capture,
    other: &LinkEndpoint,
) -> Result<bilinker::composed::Split> {
    let e = bl.endpoint.get(n);
    if matches!(other, LinkEndpoint::Path(_) | LinkEndpoint::Repo(_) | LinkEndpoint::Abstract) {
        bail!("la otra punta lleva una copia de esta aceptación");
    }
    let Some(name) = &e.r#as else { bail!("no tiene `as`: nadie declara sus partes") };
    let generator = bilinker::capture::generator_named(name)?;
    let [accepted] = e.accepted.as_slice() else {
        bail!("tiene {} aceptaciones, y hace falta una", e.accepted.len());
    };
    if accepted.link.as_ref() != Some(&e.link) {
        bail!("no está en la ubicación aprobada");
    }
    bilinker::composed::split(layer, cap, generator.as_ref(), &accepted.hash)
}

pub fn run(layer: &Path, dry_run: bool) -> Result<Outcome> {
    let plan = plan(layer)?;
    if !plan.refused.is_empty() {
        bail!("no se migra nada: {} endpoint(s) compuesto(s) no se pueden partir\n  {}",
              plan.refused.len(),
              plan.refused.iter().map(|(at, why)| format!("{at}  {why}")).collect::<Vec<_>>().join("\n  "));
    }
    let mut out = Outcome::default();
    if plan.migrated.is_empty() {
        return Ok(out);
    }

    if !dry_run {
        let src = layer.join(".bilink");
        let dst = layer.join(OUT_DIR);
        if dst.exists() {
            std::fs::remove_dir_all(&dst).with_context(|| format!("limpiando {}", dst.display()))?;
        }
        crate::accepted_list::copiar_arbol_sin_derivados(&src, &dst)?;
        for (name, text) in &plan.files {
            std::fs::write(dst.join(name), text)?;
        }
        std::fs::create_dir_all(dst.join("capture"))?;
        for (id, c) in &plan.captures {
            std::fs::write(dst.join("capture").join(format!("{id}.yaml")), c.to_yaml()?)?;
        }
    }
    out.changed = plan.files.keys().map(|n| layer.join(OUT_DIR).join(n)).collect();
    out.notes.push(plan.summary());
    Ok(out)
}

impl Plan {
    pub fn summary(&self) -> String {
        format!("{} endpoint(s) compuesto(s) partido(s) en ancla y dimensiones, {} capture(s) de nodo",
                self.migrated.len(), self.captures.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bilinker::{grammar, hash, query};
    use tempfile::tempdir;

    const CTL: &str = "\
@RestController
@RequestMapping(\"/public-api/user\")
public class Service {

    @GetMapping(\"/permissions/from-token\")
    public List<PublicAuthorityDto> getPermissions(String token)
    {
        return svc.permissionsOf(token);
    }
}
";

    const OLD: &str = r#"(class_declaration
  (modifiers
    (_
          name: (identifier) @n0 (#match? @n0 "^(RequestMapping)$")) @target)
  body: (class_body
    (method_declaration
      (modifiers
        (_
          name: (identifier) @n1 (#match? @n1 "^(GetMapping|PostMapping|PutMapping|DeleteMapping|PatchMapping|RequestMapping)$")) @target)
      type: (_) @target
      name: (identifier) @n2 (#eq? @n2 "getPermissions")
      parameters: (_) @target)))"#;

    const UUID: &str = "aaaa1111-0000-4000-8000-000000000001";

    /// Una capa 4.1.0 con un endpoint compuesto `OK`, como los de sge.
    fn layer(other: &str) -> (tempfile::TempDir, String) {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("Service.java"), CTL).unwrap();
        let old = Capture { file: "Service.java".into(), query: Some(OLD.into()) };
        let (id, _, _) = old.write_in(d.path()).unwrap();
        std::fs::write(d.path().join(".bilink/version"), "4.1.0\n").unwrap();

        let language = grammar::for_language("java").unwrap();
        let f = query::find_fragment(language, CTL, OLD).unwrap().unwrap();
        let bl = format!("\
kind: governs
endpoint:
  0:
    link: capture {id}
    accepted:
    - agree:
      - anibal
      link: capture {id}
      hash: {}
      hash_ast: {}
      n: declined
    name: back
    as: spring-controller
  1:
    link: {other}
",
            hash::sha256(f.ranges.text(CTL).as_bytes()), hash::sha256(f.sexp.as_bytes()));
        std::fs::write(d.path().join(format!(".bilink/{UUID}.yaml")), bl).unwrap();
        (d, id)
    }

    fn migrated(d: &Path) -> BiLink {
        BiLink::load(&d.join(OUT_DIR).join(format!("{UUID}.yaml"))).unwrap()
    }

    /// El endpoint queda anclado en el método, con las dimensiones del generador y
    /// la aceptación repartida entre ellas, y lo que no es del contenido, igual.
    #[test]
    fn a_composed_endpoint_becomes_anchor_and_dimensions() {
        let (d, old_id) = layer("issue 3a");
        run(d.path(), false).unwrap();
        let bl = migrated(d.path());
        let e = bl.endpoint.get(0);

        let id = e.link.capture_id().unwrap();
        assert_ne!(id, old_id);
        assert!(d.path().join(OUT_DIR).join("capture").join(format!("{id}.yaml")).exists());
        assert_eq!(e.dimensions.keys().collect::<Vec<_>>(), ["parameters", "route", "type"]);

        let a = &e.accepted[0];
        assert_eq!(a.link.as_ref(), Some(&e.link));
        assert_eq!(a.dimensions.keys().collect::<Vec<_>>(), ["parameters", "route", "type"]);
        assert_eq!(a.dimensions["type"].hash, hash::sha256(b"List<PublicAuthorityDto>"));
        assert_eq!(a.agree.iter().collect::<Vec<_>>(), ["anibal"]);
        assert!(a.n.is_some(), "el vecindario queda como estaba");
        assert_eq!(e.r#as.as_deref(), Some("spring-controller"));
        assert_eq!(e.name.as_deref(), Some("back"));
        assert_eq!(bl.kind.as_deref(), Some("governs"));
    }

    /// El capture viejo queda, sin referentes: lo saca `capture prune`.
    #[test]
    fn the_old_capture_stays() {
        let (d, old_id) = layer("issue 3a");
        run(d.path(), false).unwrap();
        assert!(d.path().join(OUT_DIR).join("capture").join(format!("{old_id}.yaml")).exists());
        assert_eq!(std::fs::read_to_string(d.path().join(OUT_DIR).join("version")).unwrap(), "4.1.0\n");
    }

    /// Todo o nada: uno que no está `OK` frena la capa entera, y se lo nombra.
    #[test]
    fn one_endpoint_that_is_not_ok_stops_the_layer() {
        let (d, _) = layer("issue 3a");
        std::fs::write(d.path().join("Service.java"), CTL.replace("String token", "String t")).unwrap();
        let e = run(d.path(), false).err().unwrap().to_string();
        assert!(e.contains("aaaa1111.0") && e.contains("no es el aceptado"), "{e}");
        assert!(!d.path().join(OUT_DIR).exists(), "no se escribe nada");
    }

    /// Otra capa lleva una copia de esta aceptación: partirla la dejaría sucia allá.
    #[test]
    fn an_acceptance_copied_by_another_layer_is_not_split() {
        let (d, _) = layer("path >impl");
        let e = run(d.path(), false).err().unwrap().to_string();
        assert!(e.contains("copia de esta aceptación"), "{e}");
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let (d, _) = layer("issue 3a");
        let o = run(d.path(), true).unwrap();
        assert_eq!(o.notes, ["1 endpoint(s) compuesto(s) partido(s) en ancla y dimensiones, 1 capture(s) de nodo"]);
        assert!(!d.path().join(OUT_DIR).exists());
    }

    /// Una capa sin captures compuestos, o ya migrada, no tiene nada que hacer.
    #[test]
    fn a_layer_without_composed_captures_is_a_no_op() {
        let (d, _) = layer("issue 3a");
        run(d.path(), false).unwrap();
        let live = d.path().join(".bilink");
        std::fs::remove_dir_all(&live).unwrap();
        std::fs::rename(d.path().join(OUT_DIR), &live).unwrap();
        assert!(run(d.path(), false).unwrap().is_empty(), "migrar dos veces no hace nada");
    }

    /// La versión no dice si le toca: lo dice su carpeta. Y el corte deja la capa
    /// migrada, con lo anterior en su backup.
    #[test]
    fn the_cut_is_chosen_by_its_folder() {
        let (d, _) = layer("issue 3a");
        let layers = vec![d.path().to_path_buf()];
        assert!(crate::cut::cuts_for(&layers).is_empty(), "sin carpeta no hay corte");

        run(d.path(), false).unwrap();
        let cuts = crate::cut::cuts_for(&layers);
        assert_eq!(cuts[0].1.out_dir, OUT_DIR);
        let c = crate::cut::plan_cut_of(d.path(), &cuts[0].1).unwrap();
        crate::cut::execute(&c).unwrap();

        assert!(d.path().join(BACKUP_DIR).join(format!("{UUID}.yaml")).exists());
        assert!(!BiLink::load(&d.path().join(format!(".bilink/{UUID}.yaml"))).unwrap()
            .endpoint.get(0).dimensions.is_empty());
        assert!(plan(d.path()).unwrap().migrated.is_empty(), "ya no queda nada compuesto");
    }

    /// Si entre generar y cortar el archivo dejó de ser el aprobado, el corte se niega.
    #[test]
    fn the_cut_verifies_again() {
        let (d, _) = layer("issue 3a");
        run(d.path(), false).unwrap();
        std::fs::write(d.path().join("Service.java"), CTL.replace("String token", "String t")).unwrap();
        let e = crate::cut::plan_cut_of(d.path(), &crate::cut::dimensions_cut()).err().unwrap();
        assert!(e.to_string().contains("aaaa1111.0"), "{e}");
    }
}
