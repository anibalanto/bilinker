//! `bilinker-003-accepted-list` — del formato 3.8 al 4.0.
//!
//! Dos cambios de forma, y uno de los dos no se puede completar acá.
//!
//! **`accepted` pasa de objeto a lista**, y eso es mecánico: un objeto se vuelve una
//! lista de uno y no se pierde nada. Un endpoint con una sola decisión sigue teniendo
//! una sola decisión.
//!
//! **`n` gana `link`**, y ahí no hay nada que traer. Los `n` ya escritos salieron de
//! hashear ubicaciones crudas; para convertirlos en captures hay que **resolver los
//! tipos de la firma**, que necesita un language server. Y una migración es *"una
//! función pura de los archivos de entrada: no consulta git, no resuelve queries
//! tree-sitter, no lee la hora"* — así que no puede, y no debería.
//!
//! # Por qué `declined` y no descartarlos
//!
//! Se consideró descartar el `n`, y es peor: bajaría la cobertura de 98 endpoints de
//! `hsi` y 15 del impl **en silencio**. Escribir `declined` deja la renuncia
//! **escrita**, que es exactamente la distinción que `2r` compró — y con eso:
//!
//! - `check` y `status` dicen que ese endpoint no vigila su vecindario, en vez de
//!   dejar creer que sí;
//! - un `accept` posterior **con** proveedor la levanta solo, porque una renuncia
//!   anterior se levanta sola en cuanto hay con qué resolver;
//! - y por `3h` nadie tiene que volver a tipear `--no-n1` en el medio.
//!
//! La otra salida —negarse si hay algún `n` adquirido— dejaría la migración bloqueada
//! por algo que la migración no puede arreglar, y obligaría a re-aceptar 113
//! endpoints **antes** de poder leer los archivos con el binario nuevo. Al revés
//! funciona: se migra, y quien quiera el vecindario lo recupera aceptando.
//!
//! # El conteo no es cosmético
//!
//! Cuántos `n` se degradaron va en las notas del ledger. Una renuncia masiva escrita
//! sin decirlo sería indistinguible de 113 personas que decidieron renunciar, y esa
//! confusión es la que el campo existe para no tener.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use accreta_migrate::Outcome;
use serde::{Deserialize, Serialize};

/// La forma **3.8**, con lo justo para leerla.
///
/// **Structs locales y no un crate congelado.** El formato 1 pedía un crate propio
/// porque era otra serialización entera; 3.8 y 4.0 son el mismo YAML y difieren en
/// dos lugares. Un crate para eso sería más código que el que puentea.
///
/// Lo que **no** está acá es todo lo que no cambia: se lee como `serde_yaml::Value` y
/// se copia. Enumerar los campos que quedan igual es lo que hace que una migración
/// se rompa con el próximo campo aditivo.
mod v38 {
    use super::*;

    #[derive(Deserialize)]
    pub struct BiLink {
        #[serde(default)]
        pub kind: Option<String>,
        pub endpoint: BTreeMap<String, Endpoint>,
    }

    #[derive(Deserialize)]
    pub struct Endpoint {
        pub link: String,
        #[serde(default)]
        pub accepted: Option<Accepted>,
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub r#as: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Accepted {
        #[serde(default)]
        pub agree: Vec<String>,
        #[serde(default)]
        pub link: Option<String>,
        pub hash: String,
        #[serde(default)]
        pub hash_ast: Option<String>,
        /// `declined` o un mapa de niveles. Se lee crudo: lo único que importa es si
        /// hay algún nivel adquirido, y eso se ve sin modelarlo.
        #[serde(default)]
        pub n: Option<serde_yaml_ng::Value>,
    }
}

/// La forma **4.0**, la que se escribe.
mod v40 {
    use super::*;

    #[derive(Serialize)]
    pub struct BiLink {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub kind: Option<String>,
        pub endpoint: BTreeMap<String, Endpoint>,
    }

    #[derive(Serialize)]
    pub struct Endpoint {
        pub link: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pub accepted: Vec<Accepted>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub r#as: Option<String>,
    }

    #[derive(Serialize)]
    pub struct Accepted {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pub agree: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub link: Option<String>,
        pub hash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub hash_ast: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub n: Option<String>,
    }
}

pub const OUT_DIR: &str = ".bilink-migrate-003-accepted-list";

pub fn run(layer: &Path, dry_run: bool) -> Result<Outcome> {
    let src = layer.join(".bilink");
    if !src.exists() {
        return Ok(Outcome::default());
    }
    let plan = plan(layer)?;
    let mut out = Outcome::default();

    if !dry_run {
        let dst = layer.join(OUT_DIR);
        if dst.exists() {
            std::fs::remove_dir_all(&dst)
                .with_context(|| format!("limpiando {}", dst.display()))?;
        }
        std::fs::create_dir_all(&dst)?;
        for (name, text) in &plan.files {
            std::fs::write(dst.join(name), text)?;
        }
        // **Un `.bilink/` a medias no es un `.bilink/`.**
        //
        // El corte *reemplaza* la carpeta viva por ésta, así que lo que no esté acá
        // desaparece. Los bilinks son lo único que esta migración transforma, pero la
        // carpeta lleva más: los captures, la versión, la regla de git y la
        // procedencia del árbol.
        //
        // Se descubrió cortando: la carpeta migrada tenía sólo los 206 bilinks, el
        // corte se llevó los 199 captures al backup, y **los 206 endpoints quedaron
        // `UNRESOLVED`** — un vínculo apuntando a un capture que ya no está.
        copiar_lo_que_no_cambia(&src, &dst)?;
        // La versión sí cambia, y es lo que esta migración existe para mover.
        std::fs::write(dst.join(bilink_format::VERSION_FILE),
                       format!("{}\n", bilink_format::VERSION))?;
    }
    out.changed = plan.files.keys().map(|n| layer.join(OUT_DIR).join(n)).collect();
    out.notes.push(plan.summary());
    Ok(plan_outcome(out))
}

fn plan_outcome(o: Outcome) -> Outcome { o }

#[derive(Default)]
pub struct Plan {
    pub files: BTreeMap<String, String>,
    /// Cuántos `accepted` se envolvieron en una lista de uno.
    pub wrapped: usize,
    /// Cuántos vecindarios **adquiridos** pasaron a `declined`.
    ///
    /// **Se cuenta y se reporta.** Una renuncia masiva escrita sin decirlo sería
    /// indistinguible de otras tantas personas que decidieron renunciar.
    pub declined: usize,
}

impl Plan {
    pub fn summary(&self) -> String {
        let mut s = format!("{} aceptación(es) envueltas en lista", self.wrapped);
        if self.declined > 0 {
            s.push_str(&format!(
                "; **{} vecindario(s) pasaron a `declined`** — sus captures no se pueden \
                 derivar sin un language server. Recuperarlos es aceptar con `lspd` vivo.",
                self.declined));
        }
        s
    }
}

/// Copia lo que la migración no transforma: `capture/`, la regla de git, la
/// procedencia.
///
/// **No enumera lo que hay que copiar: copia lo que no reconoce.** Enumerar es lo que
/// hace que la próxima cosa que viva en `.bilink/` se pierda en el próximo corte.
fn copiar_lo_que_no_cambia(src: &Path, dst: &Path) -> Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        let n = name.to_string_lossy();
        // Los bilinks los escribió la migración, y la versión se escribe aparte.
        if n.ends_with(".yaml") && !n.starts_with('.') { continue }
        if n == bilink_format::VERSION_FILE { continue }
        // La cache y el índice son derivados y no se versionan: no vale la pena
        // arrastrarlos, y `check` los regenera.
        if n == "cache" || n == "index" { continue }
        let to = dst.join(&name);
        if e.file_type()?.is_dir() { copiar_arbol(&e.path(), &to)?; }
        else { std::fs::copy(e.path(), &to)?; }
    }
    Ok(())
}

fn copiar_arbol(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() { copiar_arbol(&e.path(), &to)?; }
        else { std::fs::copy(e.path(), &to)?; }
    }
    Ok(())
}

pub fn plan(layer: &Path) -> Result<Plan> {
    let mut p = Plan::default();
    let dir = layer.join(".bilink");

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|f| f.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .filter(|f| !f.file_name().and_then(|n| n.to_str())
                      .map(|n| n.starts_with('.')).unwrap_or(false))
        .collect();
    // Orden estable: el plan tiene que ser el mismo entre corridas.
    files.sort();

    for path in &files {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("leyendo {}", path.display()))?;
        let old: v38::BiLink = serde_yaml_ng::from_str(&text)
            .with_context(|| format!("parseando {} como formato 3.8", path.display()))?;

        let mut endpoint = BTreeMap::new();
        for (k, e) in old.endpoint {
            let accepted = match e.accepted {
                None => Vec::new(),
                Some(a) => {
                    p.wrapped += 1;
                    // **Un vecindario adquirido es un mapa; una renuncia es el string
                    // `declined`.** Se distingue por la forma, sin modelar los niveles:
                    // lo único que hay que decidir es si había algo que no se puede
                    // traer.
                    let n = match &a.n {
                        Some(serde_yaml_ng::Value::Mapping(_)) => {
                            p.declined += 1;
                            Some("declined".to_string())
                        }
                        Some(serde_yaml_ng::Value::String(s)) => Some(s.clone()),
                        _ => None,
                    };
                    vec![v40::Accepted {
                        agree: a.agree, link: a.link,
                        hash: a.hash, hash_ast: a.hash_ast, n,
                    }]
                }
            };
            endpoint.insert(k, v40::Endpoint {
                link: e.link, accepted, name: e.name, r#as: e.r#as,
            });
        }

        let nuevo = v40::BiLink { kind: old.kind, endpoint };
        let name = path.file_name().context("el bilink no tiene nombre")?
            .to_string_lossy().into_owned();
        p.files.insert(name, serde_yaml_ng::to_string(&nuevo)?);
    }

    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn layer(bilink: &str) -> tempfile::TempDir {
        let d = tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".bilink")).unwrap();
        std::fs::write(d.path().join(".bilink/7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.yaml"), bilink).unwrap();
        d
    }

    /// **Un objeto se vuelve una lista de uno, y no se pierde nada.**
    #[test]
    fn an_accepted_becomes_a_list_of_one() {
        let d = layer("endpoint:\n  '0':\n    link: capture aaa\n    accepted:\n      agree:\n      - ana\n      link: capture aaa\n      hash: deadbeef\n  '1':\n    link: path >impl\n");
        let p = plan(d.path()).unwrap();
        let y = p.files.values().next().unwrap();
        // Lista, en cualquiera de las dos sangrías que YAML acepta para una
        // secuencia bajo una clave: son el mismo documento.
        assert!(y.contains("- agree:"), "una lista:\n{y}");
        assert!(y.contains("ana"), "y el agree viaja:\n{y}");
        assert_eq!(p.wrapped, 1);
        assert_eq!(p.declined, 0, "sin vecindario no hay nada que degradar");
    }

    /// **Un vecindario adquirido pasa a `declined`, y se cuenta.**
    ///
    /// Sus captures no se pueden derivar acá —haría falta un language server— y
    /// descartarlo en silencio bajaría la cobertura sin que nadie lo sepa. `declined`
    /// deja la renuncia escrita, y un `accept` con proveedor la levanta sola.
    #[test]
    fn an_acquired_neighbourhood_becomes_a_written_decline() {
        let d = layer("endpoint:\n  '0':\n    link: capture aaa\n    accepted:\n      link: capture aaa\n      hash: deadbeef\n      n:\n        1:\n          hash: 96c765b9\n          hash_ast: 88e834c4\n  '1':\n    link: path >impl\n");
        let p = plan(d.path()).unwrap();
        let y = p.files.values().next().unwrap();
        assert!(y.contains("n: declined"), "la renuncia queda escrita:\n{y}");
        assert!(!y.contains("96c765b9"), "y el fold viejo no se arrastra:\n{y}");
        assert_eq!(p.declined, 1);
        assert!(p.summary().contains("declined"), "y se reporta: {}", p.summary());
    }

    /// Una renuncia que ya estaba sigue siendo la misma, y **no se cuenta**: nadie
    /// bajó nada.
    #[test]
    fn an_existing_decline_is_carried_and_not_counted() {
        let d = layer("endpoint:\n  '0':\n    link: capture aaa\n    accepted:\n      link: capture aaa\n      hash: deadbeef\n      n: declined\n  '1':\n    link: path >impl\n");
        let p = plan(d.path()).unwrap();
        assert!(p.files.values().next().unwrap().contains("n: declined"));
        assert_eq!(p.declined, 0, "ya estaba renunciado: no lo bajó esta migración");
    }

    /// Sin `accepted` no hay nada que envolver, y el endpoint sigue en `PENDING`.
    #[test]
    fn a_pending_endpoint_stays_pending() {
        let d = layer("endpoint:\n  '0':\n    link: capture aaa\n  '1':\n    link: abstract\n");
        let p = plan(d.path()).unwrap();
        let y = p.files.values().next().unwrap();
        assert!(!y.contains("accepted"), "la lista vacía no se escribe:\n{y}");
        assert_eq!(p.wrapped, 0);
    }

    /// **Y lo que la migración escribe lo lee el formato nuevo.**
    ///
    /// Es la verificación que una migración se debe a sí misma: sin esto el puente
    /// podría producir algo sintácticamente plausible que el destino rechaza.
    #[test]
    fn what_it_writes_parses_as_4_0() {
        let d = layer("endpoint:\n  '0':\n    link: capture aaa\n    accepted:\n      agree:\n      - ana\n      link: capture aaa\n      hash: deadbeef\n      n:\n        1:\n          hash: 96c765b9\n  '1':\n    link: path >impl\n");
        let p = plan(d.path()).unwrap();
        let y = p.files.values().next().unwrap();
        let bl: bilink_format::BiLink = serde_yaml_ng::from_str(y)
            .unwrap_or_else(|e| panic!("el formato nuevo tiene que leerlo: {e}\n{y}"));
        assert_eq!(bl.endpoint.get(0).accepted.len(), 1);
        assert_eq!(bl.endpoint.get(0).accepted[0].n, Some(bilink_format::N::declined()));
    }
}

#[cfg(test)]
mod carpeta_completa_tests {
    use super::*;
    use tempfile::tempdir;

    /// **El corte reemplaza la carpeta, así que lo que no esté se pierde.**
    ///
    /// Se descubrió cortando accreta: la carpeta migrada tenía sólo los 206 bilinks,
    /// el corte se llevó los 199 captures al backup, y los 206 endpoints quedaron
    /// `UNRESOLVED` — un vínculo apuntando a un capture que ya no está.
    #[test]
    fn the_migrated_folder_carries_everything_the_live_one_had() {
        let d = tempdir().unwrap();
        let bl = d.path().join(".bilink");
        std::fs::create_dir_all(bl.join("capture")).unwrap();
        std::fs::create_dir_all(bl.join("cache")).unwrap();
        std::fs::write(bl.join("7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.yaml"),
            "endpoint:\n  '0':\n    link: capture aaa\n  '1':\n    link: abstract\n").unwrap();
        std::fs::write(bl.join("capture/aaa.yaml"), "file: Svc.rs\n").unwrap();
        std::fs::write(bl.join(".gitignore"), "cache/\nindex/\n").unwrap();
        std::fs::write(bl.join("head"), "branch main\ncommit abc\n").unwrap();
        std::fs::write(bl.join("version"), "3.8.0\n").unwrap();
        std::fs::write(bl.join("cache/state"), "derivado\n").unwrap();

        run(d.path(), false).unwrap();
        let out = d.path().join(OUT_DIR);

        assert!(out.join("capture/aaa.yaml").exists(), "los captures viajan");
        assert!(out.join(".gitignore").exists(), "y la regla de git");
        assert!(out.join("head").exists(), "y la procedencia del árbol");
        assert_eq!(std::fs::read_to_string(out.join("version")).unwrap().trim(),
                   bilink_format::VERSION, "la versión sí cambia");
        // La cache es un derivado y `check` la regenera: arrastrarla sería copiar
        // conclusiones viejas sobre archivos nuevos.
        assert!(!out.join("cache").exists(), "los derivados no viajan");
    }
}
