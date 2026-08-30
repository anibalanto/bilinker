//! Los estados de un endpoint y de un capture.
//!
//! **No son formato.** Son el resultado de una verificación: `check` los deriva
//! resolviendo la query y comparando contra `accepted`, y los escribe en la
//! [cache](crate::cache). Por eso viven acá y no en `bilink-format`, que sólo
//! define lo que está en los archivos versionados.

use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Result};

/// ¿Dónde está el fragmento? Se evalúa sin ninguna aceptación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    /// La query matchea.
    Resolved,
    /// El archivo cambió de path (git rename ≥ 50%).
    Moved,
    /// Anchor renombrado; el fragmento se localizó bajo otro nombre por similitud.
    Reanchored,
    /// La query no matchea y el anchor no se localiza.
    Unanchored,
    /// El archivo no existe; eliminación rastreable en git.
    Deleted,
    /// El archivo no se puede leer o parsear.
    Broken,
}

/// ¿Lo que hay coincide con lo que se aprobó?
///
/// Un endpoint puede desalinearse en **dos dimensiones** —dónde está y qué dice— y
/// los estados las distinguen porque se aprueban por separado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointState {
    /// `accepted` ausente. Nadie aprobó nada todavía.
    Pending,
    /// La ubicación y el contenido coinciden con lo aceptado.
    Ok,
    /// `link` ≠ `accepted.link`: la ubicación cambió y nadie la aprobó.
    ///
    /// Es la contrapartida de que `apply` ya no devuelva un endpoint a `Ok`. Mover
    /// un vínculo a otro fragmento es una decisión, y una decisión sin aprobar es
    /// trabajo pendiente: sale con 1.
    Relocated,
    /// Lo aceptado está en otro offset del nodo.
    Displaced,
    /// El fragmento contiene lo aceptado verbatim y algo más.
    Expanded,
    /// El texto difiere pero el AST coincide — sólo formato.
    Restyled,
    /// El contenido cambió estructuralmente.
    Altered,
    /// El capture referenciado no resuelve. El detalle lo da el capture.
    Unresolved,
    /// Sólo endpoint `path`: la capa apuntada no existe todavía.
    Todo,
    /// Sólo endpoint `path`: el vecino fue re-aceptado.
    ChainDirty,
    /// La capa desapareció, o el vecino no tiene endpoint estructural aceptado.
    Broken,
}

impl EndpointState {
    /// Los dos endpoints en `Ok`. **Decide qué se imprime.**
    ///
    /// Distinto de `is_clean`, que decide el código de salida: un endpoint con fix
    /// disponible no está `Ok` —hay trabajo— pero no obliga a fallar.
    pub fn is_ok(&self) -> bool { *self == Self::Ok }

    /// No hace fallar a `check`.
    ///
    /// **`Relocated` no está acá.** Antes `Moved` y `Displaced` salían con 0 porque
    /// `apply` los cerraba solo; ahora repuntar no aprueba, y un vínculo apuntando a
    /// un fragmento que nadie miró es trabajo pendiente.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Ok | Self::Displaced | Self::Expanded | Self::Restyled)
    }

    /// Tiene una ubicación nueva que `apply` puede proponer.
    pub fn has_fix(&self) -> bool {
        matches!(self, Self::Displaced | Self::Expanded)
    }
}

impl CaptureState {
    pub fn is_resolved(&self) -> bool { *self == Self::Resolved }

    /// `apply` puede proponer una ubicación nueva para este capture.
    pub fn has_fix(&self) -> bool {
        matches!(self, Self::Moved | Self::Reanchored)
    }
}

macro_rules! state_str {
    ($t:ty, $( $variant:ident => $name:literal ),+ $(,)?) => {
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self { $( Self::$variant => $name ),+ })
            }
        }
        impl FromStr for $t {
            type Err = anyhow::Error;
            fn from_str(s: &str) -> Result<Self> {
                match s.trim() {
                    $( $name => Ok(Self::$variant), )+
                    other => bail!("estado desconocido: '{other}'"),
                }
            }
        }
    };
}

state_str!(EndpointState,
    Pending    => "PENDING",
    Ok         => "OK",
    Relocated  => "RELOCATED",
    Displaced  => "DISPLACED",
    Expanded   => "EXPANDED",
    Restyled   => "RESTYLED",
    Altered    => "ALTERED",
    Unresolved => "UNRESOLVED",
    Todo       => "TODO",
    ChainDirty => "CHAIN_DIRTY",
    Broken     => "BROKEN",
);

state_str!(CaptureState,
    Resolved   => "RESOLVED",
    Moved      => "MOVED",
    Reanchored => "REANCHORED",
    Unanchored => "UNANCHORED",
    Deleted    => "DELETED",
    Broken     => "BROKEN",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_round_trips() {
        use EndpointState::*;
        for s in [Pending, Ok, Relocated, Displaced, Expanded, Restyled,
                  Altered, Unresolved, Todo, ChainDirty, Broken] {
            assert_eq!(s.to_string().parse::<EndpointState>().unwrap(), s);
        }
        use CaptureState as C;
        for s in [C::Resolved, C::Moved, C::Reanchored, C::Unanchored, C::Deleted, C::Broken] {
            assert_eq!(s.to_string().parse::<CaptureState>().unwrap(), s);
        }
    }

    /// `RELOCATED` hace fallar a `check`: repuntar no es aprobar.
    #[test]
    fn relocated_is_not_clean() {
        assert!(!EndpointState::Relocated.is_clean());
        assert!(!EndpointState::Relocated.is_ok());
    }

    /// Lo que tiene fix se imprime pero no hace fallar.
    #[test]
    fn a_fixable_state_prints_but_does_not_fail() {
        for s in [EndpointState::Displaced, EndpointState::Expanded] {
            assert!(!s.is_ok(),    "{s} no está OK: hay trabajo");
            assert!(s.is_clean(),  "{s} tiene fix: no hace fallar");
        }
    }
}
