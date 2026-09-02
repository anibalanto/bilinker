//! `.bilink/cache/state` — todo lo que se puede reconstruir.
//!
//! El criterio es uno solo: **si un valor se puede recalcular, no va en el bilink.**
//! Lo que queda versionado es lo que nadie puede reconstruir — la declaración
//! (`link`) y la decisión (`accepted`).
//!
//! No está en git, así que estar fría es un estado **normal**: clon fresco, cambio
//! de rama, otra máquina. Adentro conviven dos clases con garantías distintas:
//!
//! - `state`, `state.N` y `range` — con cache fría **no están**: hay que correr `check`.
//! - `commit` — con cache fría **cuesta más**, nunca falta: se re-deriva a demanda.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use bilink_format::{Capture, Ranges};

use crate::state::{CaptureState, EndpointState};

/// Lo que `check` sabe de un capture: dónde cayó, y si resuelve.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaptureCache {
    /// Los rangos del fragmento, `start~end` separados por coma: uno por `@target`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// Lo que se sabe de un endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndpointCache {
    /// Estado de aceptación. Lo escribe `check`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// El commit en que el fragmento quedó con el contenido aceptado.
    ///
    /// Lo escribe `accept` y no `check`, que es raro para un derivado y es
    /// deliberado: se calcula una vez, cuando hay todo el contexto a mano. Lo que
    /// define un derivado es que se pueda reconstruir, no quién lo escribió.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Cómo se llama el fragmento en el vocabulario del generador que lo capturó.
    ///
    /// **Va por endpoint aunque se componga del fragmento**: la receta con la que se
    /// compone es el `as`, y el `as` es de una punta. Dos endpoints sobre el mismo
    /// capture pueden nombrarlo distinto, o uno nombrarlo y el otro no.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// La cache de una capa. Un archivo, no uno por bilink.
///
/// Reescritura atómica, menos inodos, y cero conflictos de merge porque no se
/// versiona.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cache {
    /// El commit de `refs/bilink/<branch>` del que salió.
    ///
    /// **Con esto la cache se invalida sola.** Sin él, una cache por capa devolvería
    /// estados de otra rama en silencio cuando el árbol cambia de rama.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_commit: Option<String>,

    /// Por id de capture.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub captures: BTreeMap<String, CaptureCache>,

    /// Por `<uuid>.<N>`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub endpoints: BTreeMap<String, EndpointCache>,
}

impl Cache {
    pub fn path_in(layer: &Path) -> PathBuf {
        layer.join(".bilink").join("cache").join("state")
    }

    /// La cache de la capa, o una vacía si está fría.
    ///
    /// Fría no es un error: es el estado de un clon fresco. Un archivo ilegible o
    /// corrupto también da vacía — la cache nunca es fuente de verdad, así que ante
    /// la duda se recalcula.
    pub fn load(layer: &Path) -> Self {
        std::fs::read_to_string(Self::path_in(layer))
            .ok()
            .and_then(|t| serde_yaml_ng::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// El commit de `refs/bilink/<branch>` al que corresponde el `.bilink/` de esta
    /// capa, leído de [`head`](crate::bilink_ref).
    ///
    /// Sale de `head` y no de git: `head` es un hecho sobre el árbol, y es
    /// exactamente la pregunta *"¿de qué commit salieron estos bilinks?"* que la
    /// cache necesita para saber si sus estados siguen valiendo. Leerlo no cuesta un
    /// proceso de git, y en una capa sin `head` —antes del corte— devuelve `None`,
    /// que deja la cache como estaba.
    pub fn ref_commit_of(layer: &Path) -> Option<String> {
        let text = std::fs::read_to_string(layer.join(".bilink").join("head")).ok()?;
        text.lines()
            .find_map(|l| l.strip_prefix("commit "))
            .map(|c| c.trim().to_string())
    }

    /// La cache, sólo si corresponde a este commit de la ref.
    ///
    /// Si no coincide, se descarta entera: describe otra rama.
    pub fn load_for(layer: &Path, ref_commit: Option<&str>) -> Self {
        let c = Self::load(layer);
        match (&c.ref_commit, ref_commit) {
            (Some(a), Some(b)) if a != b => Self::default(),
            _ => c,
        }
    }

    pub fn save(&self, layer: &Path) -> Result<()> {
        let path = Self::path_in(layer);
        std::fs::create_dir_all(path.parent().expect("cache/ tiene padre"))?;
        bilink_format::write_ignore(layer)?;
        let text = serde_yaml_ng::to_string(self).context("serializando la cache")?;
        // Escritura atómica: un `check` interrumpido no deja media cache.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn capture_state(&self, id: &str) -> Option<CaptureState> {
        self.captures.get(id)?.state.as_deref()?.parse().ok()
    }

    /// Los rangos del fragmento, uno por `@target`. Ver [`Ranges`].
    pub fn capture_ranges(&self, id: &str) -> Option<Ranges> {
        self.captures.get(id)?.range.as_deref()?.parse().ok()
    }

    pub fn set_capture(&mut self, id: &str, state: CaptureState, range: Option<&Ranges>) {
        self.captures.insert(id.to_string(), CaptureCache {
            range: range.map(|r| r.to_string()),
            state: Some(state.to_string()),
        });
    }

    pub fn endpoint_state(&self, uuid: &str, n: u8) -> Option<EndpointState> {
        self.endpoints.get(&key(uuid, n))?.state.as_deref()?.parse().ok()
    }

    pub fn set_endpoint_state(&mut self, uuid: &str, n: u8, state: EndpointState) {
        self.endpoints.entry(key(uuid, n)).or_default().state = Some(state.to_string());
    }

    /// El alias de un endpoint, si su generador supo nombrarlo en el último `check`.
    pub fn alias(&self, uuid: &str, n: u8) -> Option<&str> {
        self.endpoints.get(&key(uuid, n))?.alias.as_deref()
    }

    /// Lo escribe `check`, junto con el estado. `None` lo borra: un generador que ya
    /// no sabe nombrar —o un `as` que se sacó— no puede dejar el rótulo viejo.
    pub fn set_alias(&mut self, uuid: &str, n: u8, alias: Option<String>) {
        self.endpoints.entry(key(uuid, n)).or_default().alias = alias;
    }

    pub fn commit(&self, uuid: &str, n: u8) -> Option<&str> {
        self.endpoints.get(&key(uuid, n))?.commit.as_deref()
    }

    pub fn set_commit(&mut self, uuid: &str, n: u8, commit: &str) {
        self.endpoints.entry(key(uuid, n)).or_default().commit = Some(commit.to_string());
    }

    /// El commit del contenido aceptado, derivándolo de git si la cache no lo tiene.
    ///
    /// Es la única puerta por la que se pide: una cache fría es un estado corriente
    /// —un clon fresco, otra rama, otra máquina— y `commit` es la clase de derivado
    /// que ahí significa "más lento", no "no disponible". Pedirlo por `commit()` a
    /// secas convierte lo primero en lo segundo.
    ///
    /// Lo derivado queda memoizado: el walk cuesta un `git show` por commit, y
    /// dentro de una corrida el mismo endpoint se consulta varias veces.
    pub fn commit_or_derive(
        &mut self, layer: &Path, uuid: &str, n: u8,
        cap: &Capture, accepted_hash: &str,
    ) -> Option<String> {
        if let Some(c) = self.commit(uuid, n) { return Some(c.to_string()); }
        let derived = crate::capture::derive_commit(layer, cap, accepted_hash)?;
        self.set_commit(uuid, n, &derived);
        Some(derived)
    }
}

fn key(uuid: &str, n: u8) -> String { format!("{uuid}.{n}") }

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn a_cold_cache_is_empty_not_an_error() {
        let dir = tempdir().unwrap();
        let c = Cache::load(dir.path());
        assert!(c.captures.is_empty() && c.endpoints.is_empty());
        assert_eq!(c.endpoint_state("7f3d8e9a", 0), None);
    }

    #[test]
    fn the_cache_round_trips() {
        let dir = tempdir().unwrap();
        let mut c = Cache::default();
        c.set_capture("abc", CaptureState::Resolved, Some(&Ranges::one(10, 20)));
        c.set_endpoint_state("7f3d", 0, EndpointState::Relocated);
        c.set_commit("7f3d", 0, "deadbeef");
        c.save(dir.path()).unwrap();

        let back = Cache::load(dir.path());
        assert_eq!(back.capture_state("abc"), Some(CaptureState::Resolved));
        assert_eq!(back.capture_ranges("abc"), Some(Ranges::one(10, 20)));
        assert_eq!(back.endpoint_state("7f3d", 0), Some(EndpointState::Relocated));
        assert_eq!(back.commit("7f3d", 0), Some("deadbeef"));
    }

    /// Una cache de otra rama se descarta entera, no devuelve estados ajenos.
    #[test]
    fn a_cache_from_another_branch_is_discarded() {
        let dir = tempdir().unwrap();
        let mut c = Cache::default();
        c.ref_commit = Some("aaaa".into());
        c.set_endpoint_state("7f3d", 0, EndpointState::Ok);
        c.save(dir.path()).unwrap();

        assert_eq!(Cache::load_for(dir.path(), Some("aaaa")).endpoint_state("7f3d", 0),
                   Some(EndpointState::Ok));
        assert_eq!(Cache::load_for(dir.path(), Some("bbbb")).endpoint_state("7f3d", 0),
                   None, "la cache de otra rama no se usa");
    }

    /// Un archivo corrupto da cache fría, no un error: nunca es fuente de verdad.
    #[test]
    fn a_corrupt_cache_reads_as_cold() {
        let dir = tempdir().unwrap();
        let p = Cache::path_in(dir.path());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{{{ esto no es yaml").unwrap();
        assert!(Cache::load(dir.path()).endpoints.is_empty());
    }
}
