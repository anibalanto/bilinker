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
    /// El fragmento contiene lo aceptado verbatim y algo más.
    Expanded,
    /// El texto difiere pero el AST coincide — sólo formato.
    Restyled,
    /// El contenido cambió estructuralmente.
    Altered,
    /// El capture referenciado no resuelve. El detalle lo da el capture.
    Unresolved,
    /// El vecindario de la firma se reformateó y su AST no cambió.
    ///
    /// Los tres `Contract*` son de **un eje aparte**: no hablan del fragmento sino
    /// de [los tipos que su firma menciona](../../../concepts/accept.md). Llevan
    /// prefijo por eso — `Altered` y `ContractAltered` no son grados de lo mismo,
    /// son dos preguntas.
    ///
    /// Y sólo aparecen cuando el eje del contenido dice `Ok`: un endpoint tiene un
    /// estado y no dos, y si el fragmento mismo cambió eso se reporta y alguien va a
    /// mirar igual. Lo que este eje aporta es el caso donde **el fragmento no
    /// cambió** y aun así el contrato se movió.
    ContractRestyled,
    /// Un vecino cambió: el contrato se movió. Es el caso que motivó todo esto.
    ContractAltered,
    /// Hay vecindario aceptado y **nadie pudo resolver el de hoy**.
    ///
    /// No es que el valor difiera: es que no hay con qué compararlo. Por eso es de la
    /// familia de `LayerUnreachable` y `RemoteUnreachable` —*no pude ver el otro
    /// lado*— y **no sale con 1**: correr `check` sin daemon es un modo de operación
    /// normal, no un repo en mal estado.
    ContractUnverified,
    /// Sólo endpoint `path`: la capa apuntada no existe todavía.
    Todo,
    /// Sólo endpoint `path`: el vecino fue re-aceptado.
    ChainDirty,
    /// La capa existe y el `.bilink` del uuid no, o el vecino no tiene endpoint
    /// estructural aceptado. **Es una regresión**, y por eso no comparte nombre con
    /// las ausencias que se arreglan trayendo o declarando algo.
    Broken,
    /// Sólo endpoint `path`: la capa está **declarada y no clonada**.
    ///
    /// Es normal —trabajar sin clonar todas las capas es lo esperado— y se arregla
    /// con `stratum pull`.
    LayerUnreachable,
    /// Sólo endpoint `path`: ni declarada ni presente, **con aceptación previa**.
    ///
    /// Lo que falta es la declaración. Sin aceptación previa el estado sería `Todo`:
    /// una capa que todavía no existe es una intención, no una ausencia.
    LayerUnconfigured,
    /// Sólo endpoint repo: el clon del proveedor no está.
    ///
    /// **`check` no lo resuelve**: es masivo y no hace red. Lo trae un comando
    /// puntual, y mientras tanto esto se reporta y se sigue.
    RemoteUnreachable,
    /// Sólo endpoint repo: la otra punta **dejó de ser `abstract`**.
    ///
    /// Es un hecho distinto de que el fragmento cambió, y no se mezcla en el mismo
    /// token. El nombre describe la condición desde el lado que la sufre, que es el
    /// único que puede observarla: el proveedor no rechaza a nadie en particular,
    /// ni sabe que hay alguien.
    Rejected,
    /// Sólo endpoint `abstract`: la punta está abierta a quien la consuma.
    ///
    /// **Constante y siempre sana**: no hay contra qué compararla, así que no puede
    /// tomar otro valor. Se le da un nombre en vez de dejar el slot vacío porque la
    /// tupla `(state.0, state.1)` la consumen `check`, `accept .`, `status` y
    /// lattice: un valor constante lo maneja cada uno sin ramas, un hueco obliga a
    /// todos a tratar el caso nulo.
    Open,
}

impl EndpointState {
    /// Los dos endpoints en `Ok`. **Decide qué se imprime.**
    ///
    /// Distinto de `is_clean`, que decide el código de salida: un endpoint con fix
    /// disponible no está `Ok` —hay trabajo— pero no obliga a fallar.
    pub fn is_ok(&self) -> bool { *self == Self::Ok }

    /// No hace fallar a `check`.
    ///
    /// **`Relocated` no está acá.** Antes `Moved` salía con 0 porque `apply` lo
    /// cerraba solo; ahora repuntar no aprueba, y un vínculo apuntando a un
    /// fragmento que nadie miró es trabajo pendiente.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Ok | Self::Expanded | Self::Restyled | Self::Open
                     | Self::Todo | Self::LayerUnreachable | Self::RemoteUnreachable
                     | Self::ContractUnverified)
    }

    /// El estado es del eje del **vecindario** y no del fragmento.
    pub fn is_contract(&self) -> bool {
        matches!(self, Self::ContractRestyled | Self::ContractAltered | Self::ContractUnverified)
    }

    /// La punta abierta, que `accept .` nunca toca.
    pub fn is_open(&self) -> bool { *self == Self::Open }
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
    Expanded   => "EXPANDED",
    Restyled   => "RESTYLED",
    Altered    => "ALTERED",
    Unresolved => "UNRESOLVED",
    ContractRestyled   => "CONTRACT_RESTYLED",
    ContractAltered    => "CONTRACT_ALTERED",
    ContractUnverified => "CONTRACT_UNVERIFIED",
    Todo       => "TODO",
    ChainDirty => "CHAIN_DIRTY",
    Broken     => "BROKEN",
    LayerUnreachable  => "LAYER_UNREACHABLE",
    LayerUnconfigured => "LAYER_UNCONFIGURED",
    RemoteUnreachable => "REMOTE_UNREACHABLE",
    Rejected   => "REJECTED",
    Open       => "OPEN",
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
        for s in [Pending, Ok, Relocated, Expanded, Restyled,
                  Altered, Unresolved, Todo, ChainDirty, Broken,
                  LayerUnreachable, LayerUnconfigured, RemoteUnreachable,
                  Rejected, Open,
                  ContractRestyled, ContractAltered, ContractUnverified] {
            assert_eq!(s.to_string().parse::<EndpointState>().unwrap(), s);
        }
        use CaptureState as C;
        for s in [C::Resolved, C::Moved, C::Reanchored, C::Unanchored, C::Deleted, C::Broken] {
            assert_eq!(s.to_string().parse::<CaptureState>().unwrap(), s);
        }
    }

    /// `CONTRACT_UNVERIFIED` no hace fallar: no es que el valor difiera, es que no
    /// hay con qué compararlo. Correr `check` sin daemon es normal.
    #[test]
    fn not_being_able_to_look_is_not_a_failure() {
        assert!(EndpointState::ContractUnverified.is_clean());
        assert!(!EndpointState::ContractUnverified.is_ok());
        assert!(!EndpointState::ContractAltered.is_clean(), "el contrato movido sí falla");
        assert!(!EndpointState::ContractRestyled.is_clean());
    }

    /// `RELOCATED` hace fallar a `check`: repuntar no es aprobar.
    #[test]
    fn relocated_is_not_clean() {
        assert!(!EndpointState::Relocated.is_clean());
        assert!(!EndpointState::Relocated.is_ok());
    }

    /// Lo que no cierra solo se imprime pero no hace fallar.
    ///
    /// `EXPANDED` es "creció alrededor de lo aceptado, sin tocarlo": hay que
    /// mirarlo, pero lo aceptado sigue intacto y no es un vínculo roto.
    #[test]
    fn expanded_prints_but_does_not_fail() {
        let s = EndpointState::Expanded;
        assert!(!s.is_ok(),   "{s} no está OK: hay trabajo");
        assert!(s.is_clean(), "{s} no rompe nada: no hace fallar");
    }
}
