
mod lspd_neighbours;

use clap::{ArgAction, Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "bilinker", about = "Universal bidirectional structural references")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Crea un capture a partir de una selección, y escribe el .capture
    ///
    /// FILE es relativo a la raíz de la capa. START y END son posiciones línea:col (1-based).
    /// Imprime el UUID del capture por stdout, listo para referenciar desde un link.N.
    #[command(args_conflicts_with_subcommands = true)]
    Capture {
        #[command(subcommand)]
        sub: Option<CaptureCommand>,
        file:  Option<String>,
        start: Option<String>,
        end:   Option<String>,
        /// Mostrar el capture que se crearía sin escribir nada
        #[arg(long)]
        dry_run: bool,
    },

    /// Print content or list bilinks referencing a file/position
    ///
    /// Forms:
    ///   get <UUID>.<N>          — show fragment content at endpoint N
    ///   get <file>:<line>:<col> — list bilinks whose range covers that position
    ///   get <file>              — list all bilinks referencing that file
    Get {
        target: String,
        #[arg(short = 'B', value_name = "rows:cols")]
        before: Option<String>,
        #[arg(short = 'A', value_name = "rows:cols")]
        after: Option<String>,
        /// Mostrar diff entre el fragmento aceptado y el actual
        #[arg(long)]
        diff: bool,
        /// El texto del fragmento y nada más: sin números de línea y sin huecos
        ///
        /// Es lo que `check` hashea, byte por byte. Sirve para comparar; para leer
        /// está la vista, que es el default.
        #[arg(long)]
        raw: bool,
    },

    /// Repunta un endpoint a otro fragmento
    ///
    /// Para endpoints en UNANCHORED o REANCHORED sin fix automático: una sección
    /// renombrada, un test reescrito. Crea el capture, valida y repunta link.N.
    /// No acepta — correr `bilinker accept` después de revisar.
    Recapture {
        /// Endpoint a repuntar: UUID.N
        target: String,
        /// Archivo del fragmento nuevo, relativo a la capa
        file: String,
        /// Posición línea:col. Omitir para capturar el archivo completo
        pos: Option<String>,
        /// Fin de la selección línea:col. Default: igual que pos
        end: Option<String>,
    },

    /// Verify bilinks in a .bilink file or directory
    Check {
        /// Path a un .bilink o a una capa. Default: capa actual.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Watch for changes in linked files and alert on drift
    Watch,

    /// Apply auto-fixes for bilinks in MOVED/DISPLACED/EXPANDED/REANCHORED state
    Apply {
        #[arg(long)]
        dry_run: bool,
        #[arg(short = 'y')]
        yes: bool,
        /// Only apply fixes of this state (e.g. --filter MOVED)
        #[arg(long, value_name = "ESTADO")]
        filter: Option<String>,
    },

    /// Migra los metadatos de bilinker al formato actual
    Migrate {
        /// Capa a migrar (default: directorio actual)
        path: Option<PathBuf>,
        /// Migrar también todas las capas descendientes
        #[arg(long)]
        recursive: bool,
        /// Mostrar qué haría sin escribir nada
        #[arg(long)]
        dry_run: bool,
        /// Ejecutar el corte: regenerar, verificar, y cambiar .bilink/ por lo migrado
        #[arg(long)]
        cut: bool,
        /// Deshacer el corte: restaurar .bilink/ desde el backup
        #[arg(long)]
        rollback: bool,
    },

    /// Devolver el vecindario de nivel 1 que la migración 003 descartó
    RestoreN1 {
        /// Capa a restituir (default: directorio actual)
        path: Option<PathBuf>,
        /// Alcanzar también las capas descendientes
        #[arg(long)]
        recursive: bool,
        /// Mostrar qué escribiría sin escribir nada
        #[arg(long)]
        dry_run: bool,
        /// De dónde leer el backup (default: .bilink-formato-3/ de la capa)
        #[arg(long)]
        from: Option<PathBuf>,
    },

    /// Manage chains of bilinks
    Chain {
        #[command(subcommand)]
        sub: ChainCommand,
    },


    /// Build or check the .bilink/.index
    Index {
        #[command(subcommand)]
        sub: IndexCommand,
    },

    /// Accept bilink endpoints, establishing their hash baseline
    ///
    /// Forms (like git add):
    ///   accept .                 — all PENDING in current .bilink/
    ///   accept commands/         — PENDING endpoints pointing into that directory
    ///   accept commands/check.md — PENDING endpoints pointing to that file
    ///   accept <uuid>            — both endpoints of that UUID
    ///   accept <uuid>.<0|1>      — one specific endpoint
    /// Registra el estado actual de un endpoint como aprobado
    Accept {
        /// path, UUID, o UUID.N
        target: String,
        /// Aprueba sólo la ubicación: escribe accepted.link y deja el contenido
        #[arg(long)]
        place: bool,
        /// Aprueba sólo el contenido: escribe accepted.hash
        #[arg(long)]
        content: bool,
        /// Renuncia al vecindario entero, del nivel 1 para arriba: escribe n1: declined
        #[arg(long = "no-n1")]
        no_n1: bool,
        /// Sólo con --no-n1, y sólo donde éste baja una cobertura que ya estaba
        #[arg(long, requires = "no_n1")]
        force: bool,
    },

    /// Pone a punto el clon: exclude, refspec y materialización de .bilink/
    ///
    /// Es lo primero que corre cualquiera que vaya a usar bilinker, y sin él ningún
    /// otro comando corre. Es por repo, no por capa, y es idempotente.
    Init {
        /// Mostrar qué haría sin escribir nada
        #[arg(long)]
        dry_run: bool,
    },

    /// Alinea refs/bilink/<branch> con la rama del proyecto, absorbiéndola
    ///
    /// Cubre el caso en que el proyecto avanzó y nadie aceptó nada. No verifica
    /// nada: no corre tree-sitter, no resuelve captures y no escribe la cache. Y no
    /// publica: para eso está `bilinker push`.
    Sync {
        /// Mostrar qué haría sin escribir nada
        #[arg(long)]
        dry_run: bool,
    },

    /// Publica refs/bilink/<branch> en el remoto
    ///
    /// `git push` a secas no la empuja —está fuera de refs/heads/— y el refspec lo
    /// arma bilinker: ninguna interacción con refs/bilink/* se hace tipeando git.
    Push {
        /// Rama cuya ref publicar. Default: la rama actual
        branch: Option<String>,
        /// Remoto al cual publicar. Default: el único, u `origin`
        #[arg(long)]
        remote: Option<String>,
    },

    /// Mueve los bilinks de una capa a la de arriba
    ///
    /// Un `.bilink/` fabrica una raíz de capa. Si queda en un directorio que
    /// stratum no declara como capa, el check de arriba deja de ver esos bilinks
    /// sin decir nada. Los `hash` no cambian: lo que se mueve es la ubicación.
    Relayer {
        /// La capa a vaciar, relativa a la actual (p.ej. subsystems/stratum)
        layer: String,
        /// Muestra qué movería sin escribir nada
        #[arg(long)]
        dry_run: bool,
    },

    /// Qué le pasó a un bilink: quién aceptó qué, cuándo y contra qué código
    ///
    /// Los demás comandos miran el presente; éste mira la ref, que es donde vive el
    /// registro de decisiones. No escribe nada: arma una vista.
    History {
        /// <uuid> o <uuid>.<N>
        target: String,
        /// json, para un consumidor que no es una persona
        #[arg(long)]
        format: Option<String>,
    },

    /// Trae lo que otro aceptó en la misma rama, y lo une con lo tuyo
    ///
    /// Es el caso 3.b: los dos lados cuelgan de la misma absorción, así que el
    /// árbol de código no se elige. `adopt` es para otra rama.
    Pull {
        /// De cuál traer. Default: el único que haya, u `origin`
        remote: Option<String>,
        /// Muestra qué entraría sin escribir nada
        #[arg(long)]
        dry_run: bool,
    },

    /// Verifica que una refs/bilink/* tenga la forma que promete
    ///
    /// La misma verificación del lado del servidor —donde rechaza un push— y del
    /// lado del que recibe una ref ajena. No resuelve ninguna query y no escribe
    /// nada: es lo único que un hook puede correr sin efectos.
    VerifyRef {
        /// <viejo>..<nuevo>, o una ref. Default: la ref de la rama actual
        range: Option<String>,
        /// El allowed_signers de ssh. Sin él, la firma no se verifica y se dice
        #[arg(long)]
        signers: Option<std::path::PathBuf>,
        /// Lee "<viejo> <nuevo> <ref>" por línea: el protocolo de un pre-receive
        #[arg(long)]
        stdin: bool,
    },

    /// Crea refs/bilink/<branch> para una rama que no la tiene
    ///
    /// Hereda los bilinks del commit de otra ref cuyo commit absorbido siga siendo
    /// ancestro de esta rama, y absorbe el tip de la rama como segundo padre. Sin
    /// candidato, crea la ref desde cero con el .bilink/ del árbol — que es el corte.
    Track {
        /// Rama del proyecto a trackear
        branch: String,
        /// Heredar de la ref de esta rama, en vez de buscarla
        #[arg(long, value_name = "RAMA")]
        from: Option<String>,
    },

    /// Trae a esta rama las decisiones que otra rama aceptó
    ///
    /// Después de un rebase: el código del vecino entró a la rama, y si el vecino
    /// aceptó algo sobre ese código, los bilinks heredados reportarían drift que ya
    /// está resuelto. Asimétrico: ninguna decisión de acá va para allá.
    Adopt {
        /// Rama del proyecto de la que traer decisiones (origin/main y main son lo mismo)
        branch: String,
        /// Calcular y reportar lo mismo, sin escribir un solo archivo
        #[arg(long)]
        dry_run: bool,
    },

    /// Trae el repo de un proveedor declarado en .bilink/.{alias}.toml
    ///
    /// Es la operación de red de la frontera, y es explícita a propósito: `check`
    /// corre sobre todo y no puede clonar como efecto colateral. El clon arranca
    /// superficial y con sparse-checkout derivado de los bilinks.
    Fetch {
        /// Alias del proveedor. Sin argumento, todos los declarados
        alias: Option<String>,
    },

    /// Qué abstracciones hay para consumir, con su código
    ///
    /// Con un alias, las que publica ese proveedor — el paso previo a `chain new
    /// --from-repo`, para poder ver de qué colgarse. Sin alias, las que publica esta
    /// capa. No trae nada ni amplía el sparse: los blobs ya están en el clon.
    Abstracts {
        /// Alias del proveedor. Sin argumento, lo que publica esta capa
        alias: Option<String>,
        /// Cuántas líneas del fragmento mostrar. 0 = todas
        #[arg(short = 'n', default_value = "3")]
        lines: usize,
    },

    /// Show status of all bilinks in the current layer (like git status)
    Status {
        /// Layer directory to inspect (default: current directory)
        path: Option<PathBuf>,
        /// Qué cambió en .bilink/ respecto del commit de la ref, en vez de los estados
        ///
        /// Es el `git status` de bilinker: esos cambios no aparecen en el del
        /// proyecto, que los tiene excluidos.
        #[arg(long)]
        porcelain: bool,
    },

    /// El registro de decisiones: los commits propios de refs/bilink/<branch>
    ///
    /// Quién aceptó qué y cuándo, sin una sola línea del historial del proyecto de
    /// por medio.
    Log {
        /// Rama cuya ref leer. Default: la rama actual
        branch: Option<String>,
        /// Excluir los commits que la ref de esta otra rama ya tiene
        #[arg(long, value_name = "RAMA")]
        not: Option<String>,
    },

    /// El diff de .bilink/ contra el commit de la ref del que salió
    Diff {
        /// Commit de la ref contra el cual comparar. Default: el que head nombra
        against: Option<String>,
    },

    /// Remove a bilink file from the current layer
    Remove {
        /// UUID or 8-char prefix of the bilink to remove
        uuid: String,
    },



    /// Traverse the bilink graph from a file, position, or UUID
    Graph {
        /// File, file:line:col, or UUID
        selector: String,
        /// Maximum traversal depth (default: unlimited)
        #[arg(long)]
        depth: Option<usize>,
        /// Output format: tree, flat, json
        #[arg(long, default_value = "tree", value_name = "FORMAT")]
        format: String,
        /// Collect bilinks from all layers under the project root
        #[arg(long)]
        recursive: bool,
    },
}

#[derive(Subcommand)]
enum CaptureCommand {
    /// Elimina los captures de la capa que ningún .bilink referencia
    Prune {
        path: Option<PathBuf>,
        #[arg(short = 'y')]
        yes: bool,
    },
    /// Elimina un capture puntual
    Remove {
        /// UUID o prefijo del capture
        uuid: String,
        /// Eliminarlo aunque tenga referentes
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ChainCommand {
    /// Create a new chain or direct link
    ///
    /// Each --tip is a stratum path with optional :LINE:COL suffix.
    ///
    /// Examples:
    ///   bilinker chain new --tip commands/capture.md --tip '>impl/crates/bilinker/src/capture.rs:16:1'
    ///   bilinker chain new --tip spec/Foo.java --tip '>impl/src/Foo.java:42:5,58:5'
    New {
        /// Tip: STRATUM_PATH[:LINE:COL[,LINE:COL]...]  (specify exactly twice)
        #[arg(long = "tip", value_name = "REF", action = ArgAction::Append)]
        tip: Vec<String>,
        /// Intermediate layer (can repeat, order matters)
        #[arg(long = "mid", action = ArgAction::Append)]
        mid: Vec<String>,
        /// Consumir una abstracción de otro repo: `<alias>:<uuid>`
        ///
        /// Toma el uuid del bilink remoto en vez de generar uno, y arma el endpoint
        /// repo. Es la única forma de `chain new` que no genera uuid: el uuid
        /// compartido es lo que hace el rendezvous entre los dos repos.
        #[arg(long = "from-repo", value_name = "ALIAS:UUID")]
        from_repo: Option<String>,
        /// El `kind` del bilink: qué clase de relación declara
        #[arg(long)]
        kind: Option<String>,
        /// El `name` del endpoint 0: su rol en la relación
        #[arg(long = "name.0", value_name = "ETIQUETA")]
        name0: Option<String>,
        /// El `name` del endpoint 1
        #[arg(long = "name.1", value_name = "ETIQUETA")]
        name1: Option<String>,
        /// Qué parte del nodo señalado captura el tip 0
        #[arg(long = "as.0", value_name = "MODO")]
        as0: Option<String>,
        /// Qué parte del nodo señalado captura el tip 1
        #[arg(long = "as.1", value_name = "MODO")]
        as1: Option<String>,
        /// Lista los modos que hay y no hace nada más
        #[arg(long = "as")]
        list_modes: bool,
        /// Muestra qué capturaría y no escribe nada
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// No pregunta: para scripts y para CI
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Show complete state of a chain
    Status { uuid: String },
    /// Lista las cadenas del proyecto, con filtros que se acumulan
    List {
        /// Texto que el alias tiene que contener, sin distinguir mayúsculas
        patron: Option<String>,
        /// Con qué generador se capturó algún extremo
        #[arg(long = "as", value_name = "MODO")]
        as_: Option<String>,
        /// Qué clase de extremo tiene: capture, path, issue, abstract, repo
        #[arg(long, value_name = "TIPO")]
        link: Option<String>,
        /// El estado de la cadena
        #[arg(long, value_name = "ESTADO")]
        state: Option<String>,
        /// Que algún extremo referencie un archivo bajo este path
        #[arg(long, value_name = "PATH")]
        under: Option<String>,
    },
}

/// Los filtros de `chain list`, que **se combinan con Y**.
///
/// Que se acumulen es lo que permite bajar de 98 a una sin salir del comando: con uno
/// solo habría que elegir por cuál de las cuatro preguntas empezar.
#[derive(Default)]
struct ChainFilter {
    patron: Option<String>,
    as_:    Option<String>,
    link:   Option<String>,
    state:  Option<String>,
    under:  Option<String>,
}

impl ChainFilter {
    fn vacio(&self) -> bool {
        self.patron.is_none() && self.as_.is_none() && self.link.is_none()
            && self.state.is_none() && self.under.is_none()
    }

    /// Si esta cadena pasa todos los filtros puestos.
    ///
    /// **Los dos ejes de tipo se preguntan por separado** —`--link` es qué clase de
    /// extremo es, `--as` con qué receta se capturó— porque son independientes: un
    /// `capture` puede tener cualquier `as` o ninguno, y un `abstract` no tiene
    /// ninguno porque no captura nada.
    fn pasa(
        &self,
        alias: Option<&str>,
        estado: &str,
        nodes: &[(PathBuf, bilink_format::BiLink)],
    ) -> bool {
        let extremos = || nodes.iter().flat_map(|(_, bl)| [bl.endpoint.get(0), bl.endpoint.get(1)]);

        if let Some(t) = &self.patron {
            let Some(a) = alias else { return false };
            if !a.to_lowercase().contains(&t.to_lowercase()) { return false }
        }
        if let Some(m) = &self.as_ {
            if !extremos().any(|e| e.r#as.as_deref() == Some(m.as_str())) { return false }
        }
        if let Some(t) = &self.link {
            if !extremos().any(|e| e.link.prefix() == t) { return false }
        }
        if let Some(e) = &self.state {
            if !estado.eq_ignore_ascii_case(e) { return false }
        }
        if let Some(bajo) = &self.under {
            let mut alguno = false;
            for (layer, bl) in nodes {
                for n in [0u8, 1u8] {
                    let link = &bl.endpoint.get(n).link;
                    let Ok(Some(cap)) = bilinker::capture::capture_of(layer, link) else { continue };
                    if cap.file.starts_with(bajo.as_str()) { alguno = true; }
                }
            }
            if !alguno { return false }
        }
        true
    }
}

#[derive(Subcommand)]
enum IndexCommand {
    /// Build .bilink/.index for fast file lookups
    Build {
        path: Option<PathBuf>,
        #[arg(long)]
        recursive: bool,
    },
    /// Show index status without modifying files
    Status {
        path: Option<PathBuf>,
        #[arg(long)]
        recursive: bool,
    },
}

fn parse_pos(s: &str) -> anyhow::Result<(usize, usize)> {
    let (line, col) = s.split_once(':')
        .ok_or_else(|| anyhow::anyhow!("position must be line:col, got: {s}"))?;
    Ok((line.parse()?, col.parse()?))
}

fn project_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let (root, _) = bilinker::config::Config::load_from(cwd)?;
    Ok(root)
}

/// Parses a stratum tip: `STRATUM_PATH[:LINE:COL]`.
///
/// The stratum path encodes both layer navigation and the file.  The last
/// `Simple` token is the file; preceding tokens (`Down`, `Up`) are the layer.
/// Returns `(layer_fs_path, endpoint)` where `layer_fs_path` is relative to
/// the project root and is used both for bilink placement and as the git root.
/// Lo que un `--tip` pide, resuelto y todavía **sin escribir**.
///
/// Separar el plan de la escritura es lo que hace posible la vista previa: un
/// capture es opaco después de escrito, así que hay que poder mostrar qué agarró la
/// query mientras todavía se puede no escribirla.
struct TipPlan {
    /// La capa, relativa a la raíz del proyecto.
    layer_fs:   PathBuf,
    /// La capa, absoluta.
    layer_root: PathBuf,
    kind:       TipKind,
}

enum TipKind {
    /// La punta abierta del proveedor: no captura nada de este lado.
    Abstract,
    /// El archivo entero, sin query.
    WholeFile { file: String },
    /// N posiciones señaladas, ya resueltas a una query con N `@target`.
    ///
    /// El modo viaja con el plan: al corregir la vista en el editor hay que volver a
    /// capturar **como se pidió**, no como si no se hubiera pedido nada.
    Fragment  {
        file:    String,
        /// El nombre del generador, si se pidió uno. Viaja con el plan porque al
        /// corregir la vista en el editor hay que volver a capturar **como se
        /// pidió**, no como si no se hubiera pedido nada.
        mode:    Option<String>,
        capture: bilinker::capture::CaptureResult,
        /// Los generadores que tendrían algo que decir. Se **sugieren**: elegir por
        /// el usuario le escribiría otra cosa, y un capture es opaco después.
        suggest: Vec<&'static str>,
    },
}

impl TipPlan {
    /// La vista de lo que este tip capturaría, si captura algo.
    fn preview(&self, n: usize) -> anyhow::Result<Option<(bilinker::preview::Preview, String)>> {
        let TipKind::Fragment { file, capture, suggest, .. } = &self.kind else { return Ok(None) };
        let source = std::fs::read_to_string(self.layer_root.join(file))?;
        let label  = format!("{} :: {}", self.layer_fs.display(), file);
        // Se **sugiere** y no se elige: un generador que acierta cuando no querías
        // ya te escribió otra cosa, y un capture es opaco después.
        let note = (!suggest.is_empty()).then(|| suggest.iter()
            .map(|name| {
                let what = bilinker::capture::generators().into_iter()
                    .find(|g| g.name() == *name)
                    .map(|g| g.describe().to_string()).unwrap_or_default();
                format!("sugerencia: `--as.{n} {name}` — {what}")
            })
            .collect::<Vec<_>>().join("\n"));
        Ok(Some((
            bilinker::preview::Preview::of(&label, &source, &capture.ranges).with_note(note),
            source,
        )))
    }

    /// El mismo tip con otras posiciones — lo que devuelve una vista editada.
    fn repoint(&self, lines: &[usize]) -> anyhow::Result<TipPlan> {
        let TipKind::Fragment { file, mode, .. } = &self.kind else {
            anyhow::bail!("este tip no captura posiciones");
        };
        let sel: Vec<_> = lines.iter().map(|&l| ((l, 1), (l, 1))).collect();
        let gen = mode.as_deref().map(bilinker::capture::generator_named).transpose()?;
        let capture = bilinker::capture::capture_as(
            &self.layer_root, file, &sel, gen.as_deref())?;
        Ok(TipPlan {
            layer_fs:   self.layer_fs.clone(),
            layer_root: self.layer_root.clone(),
            kind:       TipKind::Fragment {
                file: file.clone(), mode: mode.clone(), capture, suggest: Vec::new(),
            },
        })
    }

    /// Escribe el capture y devuelve el endpoint que lo referencia.
    fn write(&self) -> anyhow::Result<bilinker::link::LinkEndpoint> {
        use bilinker::link::LinkEndpoint;
        Ok(match &self.kind {
            TipKind::Abstract => LinkEndpoint::Abstract,
            TipKind::WholeFile { file } => {
                let (uuid, _, _) = bilinker::capture::capture_file_whole(&self.layer_root, file)?;
                LinkEndpoint::Capture(uuid)
            }
            TipKind::Fragment { capture, .. } => {
                let (uuid, _, _) = capture.capture.write_in(&self.layer_root)?;
                LinkEndpoint::Capture(uuid)
            }
        })
    }
}

/// Un tip: `abstract`, o un path Stratum con cero o más posiciones.
///
/// Las posiciones extra van separadas por coma después de la primera —
/// `Foo.java:8:1,15:5` — y cada una resuelve a su nodo. De todas sale **una** query
/// con un `@target` por nodo: el patrón único es lo que las ancla entre sí.
fn plan_tip(
    root: &Path, tip_str: &str, gen: Option<&dyn bilinker::capture::CaptureGenerator>,
) -> anyhow::Result<TipPlan> {
    use stratum::PathToken;

    // `abstract` es un tip, no un path: la punta que publica el proveedor. Vive en
    // la capa actual —el bilink es suyo— y no captura nada, porque no hay fragmento
    // de este lado que aprobar.
    if tip_str.trim() == "abstract" {
        return Ok(TipPlan {
            layer_fs:   PathBuf::from("."),
            layer_root: root.to_path_buf(),
            kind:       TipKind::Abstract,
        });
    }

    let (path_str, positions) = split_tip_positions(tip_str)?;

    let tokens = stratum::parse_path(path_str)
        .map_err(|e| anyhow::anyhow!("invalid stratum path '{}': {}", path_str, e))?;

    // Last Simple token = file path; preceding tokens = layer navigation.
    let (layer_tokens, file_str) = match tokens.last() {
        Some(PathToken::Simple(p)) => {
            let layer = tokens[..tokens.len() - 1].to_vec();
            let file  = p.strip_prefix("/").unwrap_or(p).to_string_lossy().to_string();
            (layer, file)
        }
        _ => anyhow::bail!("tip must end with a file path, got: '{}'", path_str),
    };

    let layer_fs   = layer_tokens_to_fs_path(&layer_tokens)?;
    let layer_root = root.join(&layer_fs);

    let kind = if positions.is_empty() {
        if gen.is_some() {
            anyhow::bail!(
                "`--as` necesita una posición: sin señalar nada, el tip es el archivo \
                 entero y no hay nodo del que tomar una parte."
            );
        }
        TipKind::WholeFile { file: file_str }
    } else {
        let sel: Vec<_> = positions.iter().map(|&p| (p, p)).collect();
        let capture = bilinker::capture::capture_as(&layer_root, &file_str, &sel, gen)?;
        // Sin `--as`, qué generadores tendrían algo que decir. Se sugieren y nada
        // más: un generador que acierta cuando no querías ya te escribió otra cosa.
        let suggest = match gen {
            Some(_) => Vec::new(),
            None    => bilinker::capture::suggest_for(&layer_root, &file_str, positions[0])
                           .unwrap_or_default(),
        };
        TipKind::Fragment {
            file: file_str, mode: gen.map(|g| g.name().to_string()), capture, suggest,
        }
    };

    Ok(TipPlan { layer_fs, layer_root, kind })
}

/// Parte un tip en su path y sus posiciones.
///
/// La coma separa posiciones y no aparece en la primera: `Foo.java:8:1,15:5,16:5`.
/// Escribirlas todas con `:` sería ambiguo con un path que lleve números, y repetir
/// el flag ya significa *el otro extremo*.
fn split_tip_positions(tip_str: &str) -> anyhow::Result<(&str, Vec<(usize, usize)>)> {
    let mut chunks = tip_str.split(',');
    let head = chunks.next().unwrap_or(tip_str);

    // La primera posición viene pegada al path, como siempre.
    let parts: Vec<&str> = head.rsplitn(3, ':').collect();
    let (path_str, first) = if parts.len() == 3
        && parts[0].parse::<usize>().is_ok()
        && parts[1].parse::<usize>().is_ok()
    {
        (parts[2], Some((parts[1].parse::<usize>()?, parts[0].parse::<usize>()?)))
    } else {
        (head, None)
    };

    let mut positions = Vec::new();
    if let Some(p) = first { positions.push(p); }

    for chunk in chunks {
        let (line, col) = chunk.trim().split_once(':').ok_or_else(|| anyhow::anyhow!(
            "una posición extra se escribe `<línea>:<columna>`, se recibió '{chunk}'"
        ))?;
        if first.is_none() {
            anyhow::bail!(
                "'{tip_str}' tiene posiciones extra pero no la primera: \
                 se escribe `<path>:<línea>:<columna>,<línea>:<columna>`"
            );
        }
        positions.push((line.trim().parse()?, col.trim().parse()?));
    }
    Ok((path_str, positions))
}

/// Muestra qué capturaría cada tip y deja corregirlo, antes de escribir.
///
/// Devuelve si hay que escribir. Con `--dry-run` muestra y devuelve `true` para que
/// quien llama corte después; con `--yes` no pregunta.
///
/// **Sin terminal no hay a quién preguntarle**, así que se escribe: un `chain new`
/// adentro de un script no puede quedarse esperando una tecla que nadie va a
/// apretar. La confirmación existe para la persona que está mirando.
fn confirm_tips(plans: &mut [TipPlan], dry_run: bool, yes: bool) -> anyhow::Result<bool> {
    use std::io::IsTerminal;

    let mut vistas = Vec::new();
    for (i, plan) in plans.iter().enumerate() {
        if let Some((preview, source)) = plan.preview(i)? {
            vistas.push((i, preview.label.clone(), preview.render(&source)));
        }
    }
    if vistas.is_empty() { return Ok(true); }

    for (_, _, vista) in &vistas {
        eprintln!();
        eprint!("{vista}");
    }

    if yes || dry_run { return Ok(true); }
    if !std::io::stdin().is_terminal() { return Ok(true); }

    let editor = git_editor();
    let prompt = match &editor {
        Some(_) => "\n¿escribir? [y/N/e(ditar)] ",
        None    => "\n¿escribir? [y/N] ",
    };

    eprint!("{prompt}");
    use std::io::Write;
    let _ = std::io::stderr().flush();

    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer)? == 0 { return Ok(false); }

    match answer.trim() {
        "y" | "Y" => Ok(true),
        "e" | "E" if editor.is_some() => {
            // **Un solo buffer para los dos tips.** Abrir un editor por tip haría
            // corregir a ciegas el segundo: lo que se está revisando es el vínculo,
            // no cada punta por su cuenta.
            let buffer: String = vistas.iter()
                .map(|(_, _, v)| v.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let edited = edit_buffer(editor.as_deref().expect("hay editor"), &buffer)?;

            for (i, label, _) in &vistas {
                let Some(chunk) = section_of(&edited, label, &vistas) else {
                    eprintln!("falta el encabezado `{label}` en lo editado: no se escribió nada.");
                    return Ok(false);
                };
                let marks = bilinker::preview::Preview::marks_in(chunk);
                if marks.is_empty() {
                    eprintln!("`{label}` volvió sin ninguna marca: no hay qué capturar.");
                    return Ok(false);
                }
                plans[*i] = plans[*i].repoint(&marks)?;
            }
            // Lo editado se vuelve a mostrar: la corrección también se revisa.
            confirm_tips(plans, dry_run, yes)
        }
        _ => Ok(false),
    }
}

/// El tramo del buffer que corresponde a un encabezado: desde su línea hasta el
/// encabezado siguiente, o hasta el final.
fn section_of<'a>(buffer: &'a str, label: &str, all: &[(usize, String, String)]) -> Option<&'a str> {
    let start = buffer.find(label)?;
    let rest  = &buffer[start + label.len()..];
    let end = all.iter()
        .filter(|(_, l, _)| l != label)
        .filter_map(|(_, l, _)| rest.find(l.as_str()))
        .min()
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// El editor que git usaría, con su misma precedencia.
///
/// `git var GIT_EDITOR` contesta lo que git realmente va a abrir —`$GIT_EDITOR`,
/// `core.editor`, `$VISUAL`, `$EDITOR`, el fallback del sistema— en vez de leer un
/// solo lugar y acertar a veces. Es el mismo criterio por el que quién acepta sale
/// de `git var GIT_AUTHOR_IDENT` y no de `user.name`.
fn git_editor() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["var", "GIT_EDITOR"])
        .output().ok()?;
    if !out.status.success() { return None; }
    let ed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!ed.is_empty()).then_some(ed)
}

/// Abre el buffer en el editor y devuelve lo que quedó.
fn edit_buffer(editor: &str, buffer: &str) -> anyhow::Result<String> {
    let path = std::env::temp_dir().join(format!("bilinker-capture-{}.txt", std::process::id()));
    let ayuda = "\n# Las líneas con ▸ son las que se capturan. Sacá o agregá marcas y guardá.\n                 # Cada línea marcada resuelve a su nodo: marcar tres líneas de una\n                 # función marca la función una vez.\n                 # Sin ninguna marca, no se escribe nada.\n";
    std::fs::write(&path, format!("{buffer}{ayuda}"))?;

    // Vía shell, como git: `core.editor` puede llevar argumentos.
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\"", ))
        .arg(editor)
        .arg(&path)
        .status()
        .map_err(|e| anyhow::anyhow!("no se pudo abrir el editor `{editor}`: {e}"))?;
    if !status.success() {
        anyhow::bail!("el editor `{editor}` salió con error");
    }

    let edited = std::fs::read_to_string(&path)?;
    let _ = std::fs::remove_file(&path);
    Ok(edited)
}

/// Converts layer navigation tokens (Up / Down only) to a filesystem path
/// relative to the project root. `[]` → `.` (current layer).
fn layer_tokens_to_fs_path(tokens: &[stratum::PathToken]) -> anyhow::Result<PathBuf> {
    use stratum::PathToken;
    if tokens.is_empty() {
        return Ok(PathBuf::from("."));
    }
    let mut path = PathBuf::new();
    for token in tokens {
        match token {
            PathToken::Down(name)  => path = path.join(".stratum").join(name),
            PathToken::Up          => path = path.join("..").join(".."),
            // Un componente de path corriente. Sin esto no se puede atravesar un
            // directorio común antes de bajar a una capa, que es la forma de este
            // proyecto: `subsystems/bilinker>impl`.
            PathToken::Simple(p)   => path = path.join(p),
            PathToken::TopRoot     => anyhow::bail!("`*` (TopRoot) not supported in chain new tips"),
            PathToken::Root        => anyhow::bail!("`<*` (Root) not supported in chain new tips"),
        }
    }
    Ok(path)
}

fn parse_accept_target(target: &str) -> anyhow::Result<(String, u8)> {
    let dot = target.rfind('.')
        .ok_or_else(|| anyhow::anyhow!("target must be <uuid>.<0|1>, got: {target}"))?;
    let n: u8 = target[dot + 1..]
        .parse()
        .map_err(|_| anyhow::anyhow!("endpoint index must be 0 or 1, got: '{}'", &target[dot + 1..]))?;
    if n > 1 {
        anyhow::bail!("endpoint index must be 0 or 1, got: {n}");
    }
    Ok((target[..dot].to_string(), n))
}


fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;

    // `init` se configura a sí mismo; `migrate` corre en repos que todavía no
    // cortaron y no puede exigir una ref que no existe.
    //
    // Y `verify-ref` **no toca el árbol de trabajo**: verifica una ref, que puede ser
    // ajena, de otra rama, o la que un push está proponiendo. Materializar antes
    // sería escribir —lo único que un hook no puede hacer— y además fallaría justo
    // donde más se lo necesita: sobre una ref que no corresponde a este árbol.
    if !matches!(
        cli.command,
        Command::Init { .. } | Command::Migrate { .. } | Command::VerifyRef { .. }
    ) {
        match bilinker::init::prelude(&cwd)? {
            bilinker::init::Materialization::Rematerialized { from, to } => {
                eprintln!("materializado: {} → refs/bilink/… @ {}",
                          from.branch, short(&to));
            }
            bilinker::init::Materialization::Detached => {
                eprintln!("aviso: HEAD desacoplado — se opera contra lo que .bilink/head dice");
            }
            _ => {}
        }
    }

    match cli.command {
        Command::Capture { sub, file, start, end, dry_run } => {
            match sub {
                Some(CaptureCommand::Remove { uuid, force }) => {
                    let dir = bilink_format::Capture::dir(&cwd);
                    let all = bilink_format::Capture::all_in(&cwd)?;
                    let hits: Vec<_> = all.iter().filter(|(id, _)| id.starts_with(&uuid)).collect();
                    let cap = match hits.as_slice() {
                        []  => anyhow::bail!("no hay capture que empiece con '{uuid}'"),
                        [c] => *c,
                        _   => anyhow::bail!("'{uuid}' es ambiguo: {} captures coinciden", hits.len()),
                    };

                    // Un capture con referentes deja bilinks apuntando a la nada.
                    let orphan = bilinker::capture::orphans(&cwd)?.iter().any(|(id, _)| *id == cap.0);
                    if !orphan && !force {
                        anyhow::bail!(
                            "el capture {} tiene referentes — usar `bilinker recapture` para repuntarlos, o --force",
                            &cap.0[..8.min(cap.0.len())]
                        );
                    }

                    std::fs::remove_file(dir.join(format!("{}.yaml", cap.0)))?;
                    eprintln!("eliminado: {}  {}", &cap.0[..8.min(cap.0.len())], cap.1.file);
                    if !orphan {
                        eprintln!("warn: tenía referentes — correr `bilinker check .`");
                    }
                }

                Some(CaptureCommand::Prune { path, yes }) => {
                    let layer = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                        .unwrap_or_else(|| cwd.clone());
                    let orphans = bilinker::capture::orphans(&layer)?;
                    if orphans.is_empty() {
                        eprintln!("no hay captures sin referentes");
                        return Ok(());
                    }
                    println!("{} capture(s) sin referentes:", orphans.len());
                    for c in &orphans {
                        // La query es multilínea; en una lista de confirmación
                        // previa a borrar archivos, una línea por capture.
                        let anchor = match c.1.query.as_deref() {
                            None => "archivo completo".to_string(),
                            Some(q) => {
                                let kind = q.split_whitespace().next().unwrap_or("")
                                    .trim_start_matches('(');
                                let name = q.split("#eq?").nth(1)
                                    .and_then(|t| t.split('"').nth(1)).unwrap_or("");
                                format!("{kind} {name}").trim().to_string()
                            }
                        };
                        println!("  {}…  {}  [{anchor}]",
                                 &c.0[..8.min(c.0.len())], c.1.file);
                    }
                    if !yes {
                        eprint!("
Eliminar? [y/N] ");
                        use std::io::Write;
                        std::io::stderr().flush().ok();
                        let mut input = String::new();
                        std::io::stdin().read_line(&mut input)?;
                        if input.trim().to_lowercase() != "y" {
                            eprintln!("abortado");
                            return Ok(());
                        }
                    }
                    for c in &orphans {
                        std::fs::remove_file(
                            bilink_format::Capture::path_in(&layer, &c.0))?;
                    }
                    eprintln!("eliminados {} capture(s)", orphans.len());
                }

                None => {
                    let Some(file) = file else {
                        anyhow::bail!("uso: bilinker capture <file> [<start> <end>]");
                    };
                    let root = project_root(&cwd)?;

                    // Sin selección, el fragmento es el archivo entero: no hay nodo
                    // que buscar, así que tampoco ancla que verificar ni gramática
                    // que haga falta. Es la forma más usada del lado de las specs.
                    let (Some(start), Some(end)) = (start, end) else {
                        if dry_run {
                            eprintln!("[dry-run] no se escribió nada");
                            eprintln!("file:   {file}");
                            eprintln!("query:  (ausente — el archivo completo)");
                            return Ok(());
                        }
                        let (uuid, path, reused) =
                            bilinker::capture::capture_file_whole(&root, &file)?;
                        println!("{uuid}");
                        eprintln!("{}: {}", if reused { "reusado" } else { "creado" }, path.display());
                        return Ok(());
                    };
                    let (s, e) = (parse_pos(&start)?, parse_pos(&end)?);

                    if dry_run {
                        let result = bilinker::capture::capture(&root, &file, s, e)?;
                        eprintln!("[dry-run] no se escribió nada");
                        eprintln!("file:   {}", result.capture.file);
                        if let Some(q) = &result.capture.query {
                            eprintln!("query:  {q}");
                        }
                        return Ok(());
                    }
                    let (uuid, path, reused) =
                        bilinker::capture::capture_to_file(&root, &file, s, e)?;
                    println!("{uuid}");
                    if reused {
                        eprintln!("reusado: {}", path.display());
                        eprintln!("  ya existía un capture con esta misma referencia");
                    } else {
                        eprintln!("creado: {}", path.display());
                    }
                }
            }
        }

        Command::Get { target, before, after, diff, raw } => {
            let uuid_form = {
                let t = target.trim();
                if let Some(dot) = t.rfind('.') {
                    (t.ends_with(".0") || t.ends_with(".1"))
                        && t[..dot].chars().all(|c| c.is_ascii_hexdigit() || c == '-')
                        && !t[..dot].is_empty()
                } else {
                    false
                }
            };
            let pos_form = {
                let parts: Vec<&str> = target.rsplitn(3, ':').collect();
                parts.len() == 3
                    && parts[0].parse::<usize>().is_ok()
                    && parts[1].parse::<usize>().is_ok()
            };

            if uuid_form {
                let dot      = target.rfind('.').unwrap();
                let name     = &target[..dot];
                let endpoint: u8 = target[dot + 1..].parse()?;
                let root     = project_root(&cwd)?;

                if diff {
                    let result = bilinker::get::get_diff(&root, name, endpoint)?;
                    eprintln!("# {}  lines {}–{}", result.file, result.start_line, result.end_line);
                    match &result.diff {
                        Some(d) => print!("{d}"),
                        None    => eprintln!("[sin cambios]"),
                    }
                } else {
                    let before   = before.as_deref().map(parse_pos).transpose()?;
                    let after    = after.as_deref().map(parse_pos).transpose()?;
                    let result   = bilinker::get::get(&root, name, endpoint, before, after)?;
                    // El alias contesta **qué** se está mirando; el archivo y las
                    // líneas contestan dónde. Sin alias el encabezado es el de antes.
                    if let Some(a) = bilinker::cache::Cache::load(&root).alias(name, endpoint) {
                        eprintln!("# {a}");
                    }
                    eprintln!("# {}  lines {}", result.file, result.line_span());
                    // La vista es el default: si alguien quiere el texto exacto es
                    // **para compararlo**, y comparar lo hacen `check` y `--diff`.
                    if raw { println!("{}", result.content); }
                    else   { print!("{}", result.view); }
                }
            } else if pos_form {
                let mut parts = target.rsplitn(3, ':');
                let col:  usize = parts.next().unwrap().parse()?;
                let line: usize = parts.next().unwrap().parse()?;
                let file        = parts.next().unwrap();
                let file_path   = cwd.join(file);
                let root        = project_root(&cwd)?;

                let results = bilinker::check::find_by_file(&root, &file_path)?;
                if results.is_empty() {
                    return Ok(());
                }
                for (bilink_path, n, range) in results {
                    let source = std::fs::read_to_string(&file_path).unwrap_or_default();
                    let byte = line_col_to_byte(&source, line, col);
                    if range.parts().iter().any(|r| byte >= r.start && byte < r.end) {
                        let uuid  = bilink_path.file_stem()
                            .and_then(|s| s.to_str()).unwrap_or("?");
                        let bl    = bilink_format::BiLink::load(&bilink_path)?;
                        let other = &bl.endpoint.get(1 - n).link;
                        println!("{uuid}.{n}  {other}");
                    }
                }
            } else {
                let file_path = cwd.join(&target);
                let root      = project_root(&cwd)?;

                let results = bilinker::check::find_by_file(&root, &file_path)?;
                for (bilink_path, n, range) in results {
                    let uuid  = bilink_path.file_stem()
                        .and_then(|s| s.to_str()).unwrap_or("?");
                    let bl    = bilink_format::BiLink::load(&bilink_path)?;
                    let other = &bl.endpoint.get(1 - n).link;
                    println!("{uuid}.{n}  {other}  bytes {range}");
                }
            }
        }

        Command::Recapture { target, file, pos, end } => {
            let (uuid, n) = target.rsplit_once('.')
                .and_then(|(u, n)| n.parse::<u8>().ok().map(|n| (u, n)))
                .ok_or_else(|| anyhow::anyhow!("el target debe ser UUID.N, se recibió '{target}'"))?;
            if n > 1 { anyhow::bail!("el endpoint debe ser 0 o 1"); }

            let (bilink_path, _) =
                (bilinker::accept::find_bilink_path(&cwd, uuid)?, ());

            let range = match (pos.as_deref(), end.as_deref()) {
                (None, _)          => None,
                (Some(p), None)    => { let p = parse_pos(p)?; Some((p, p)) }
                (Some(p), Some(e)) => Some((parse_pos(p)?, parse_pos(e)?)),
            };
            let r = bilinker::capture::recapture(&cwd, &bilink_path, n, &file, range)?;

            println!("{}", r.new_uuid);
            eprintln!("link.{n} → capture {}{}",
                      &r.new_uuid[..8.min(r.new_uuid.len())],
                      if r.reused { "  (reusado)" } else { "" });
            if let Some(old) = &r.old_uuid {
                eprintln!("  antes: {}{}", &old[..8.min(old.len())],
                          if r.orphaned { "  (quedó sin referentes)" } else { "" });
            }
            eprintln!("\nrevisar con `bilinker get {target}` y aceptar con `bilinker accept {target}`");
        }

        Command::Check { path } => {
            let root = project_root(&cwd)?;
            let check_path = if path.is_absolute() { path } else { cwd.join(path) };
            let checked = match bilinker::check::check_with(
                &root, &check_path, Some(&lspd_neighbours::Lspd),
            ) {
                Ok(c)  => c,
                Err(e) => {
                    // **Una versión que no se entiende sale con 2, no con 1.** No es
                    // lo mismo *"hay endpoints no-OK"* que *"no leí nada"*: un CI que
                    // trata cualquier no-cero igual no nota la diferencia, y uno que
                    // sí puede distinguir un drift que hay que revisar de una capa que
                    // hay que migrar.
                    if let Some(m) = e.downcast_ref::<bilink_format::Mismatch>() {
                        eprintln!("Error: {m}");
                        std::process::exit(2);
                    }
                    return Err(e);
                }
            };

            // Se imprime todo lo que no está OK; solo falla lo que no tiene auto-fix.
            let mut exit_code = 0;
            let mut shown     = 0;
            for r in &checked.results {
                if !r.all_ok() {
                    shown += 1;
                    println!("{}  ({}, {})", &r.uuid[..8], r.state0, r.state1);
                }
                if !r.is_clean() {
                    exit_code = 1;
                }
            }

            // **Un bilink ilegible sale con 1** por lo mismo que `PENDING`: hay
            // trabajo que hacer y nadie lo hizo. Que el archivo esté roto en vez de
            // pendiente no lo vuelve menos trabajo.
            if !checked.unreadable.is_empty() {
                exit_code = 1;
                eprintln!("\n{} bilink(s) no se pudieron leer:", checked.unreadable.len());
                for u in &checked.unreadable {
                    eprintln!("  {}  {}", u.path.display(), u.error);
                }
            }

            // `all clean` es una afirmación sobre **todo lo que hay**, así que no se
            // imprime cuando quedó algo sin leer.
            match (shown, checked.unreadable.len()) {
                (0, 0) => eprintln!("all clean ({} bilink(s))", checked.results.len()),
                (0, u) => eprintln!("\n{} bilink(s) verificados, todos OK — {u} ilegible(s)",
                                    checked.results.len()),
                // Con no-OK arriba el detalle ya está impreso, así que la línea final
                // sólo hace falta para decir sobre cuántos se dijo — y eso hace falta
                // justamente cuando quedó algo afuera.
                (_, 0) => {}
                (_, u) => eprintln!("\n{} bilink(s) verificados — {u} ilegible(s)",
                                    checked.results.len()),
            }
            std::process::exit(exit_code);
        }

        Command::Watch => {
            let root = project_root(&cwd)?;
            watch(&root)?;
        }

        Command::Apply { dry_run, yes, filter } => {
            let root   = project_root(&cwd)?;
            // **`apply` recibe el puerto.** Sin proveedor arregla lo del fragmento con
            // git y no toca el vecindario: descubrir qué tipos menciona la firma hoy
            // es lo único que un language server puede contestar.
            // **El paso 0 sale acá**: una capa fría no da una lista de fixes vacía, da
            // otra cosa, y por eso `Scan` es un enum y no un `Vec` con un flag al lado.
            let (mut fixes, unlooked) =
                match bilinker::apply::scan_fixeable(&cwd, Some(&lspd_neighbours::Lspd))? {
                    bilinker::apply::Scan::Cold { bilinks } => {
                        eprintln!("error: la capa no tiene estado calculado — {bilinks} bilinks sin mirar.");
                        eprintln!("  El vecindario se pregunta desde el rango del fragmento, y ese rango");
                        eprintln!("  todavía no se derivó.");
                        eprintln!();
                        eprintln!("  Correr primero:  bilinker check .");
                        std::process::exit(3);
                    }
                    bilinker::apply::Scan::Looked { fixes, unlooked } => (fixes, unlooked),
                };

            if let Some(ref state) = filter {
                let state_up = state.to_uppercase();
                fixes.retain(|f| f.reason == state_up);
            }

            // **Se imprime aunque no haya un solo fix**, que es justamente el caso que
            // mentía: sin esto, un endpoint que no se pudo mirar salía por el mismo
            // camino que uno que no tenía nada que arreglar.
            if !unlooked.is_empty() {
                eprintln!("Sin mirar ({}):", unlooked.len());
                for u in &unlooked {
                    eprintln!("  {}…  link.{}  {}", u.short(), u.n, u.why);
                }
                eprintln!();
            }

            if fixes.is_empty() {
                // El 2 es una afirmación sobre el árbol —*"no hay nada que arreglar"*—
                // así que sólo sale cuando hubo con qué hacerla.
                if unlooked.is_empty() {
                    eprintln!("no hay bilinks en estado auto-fixeable");
                    std::process::exit(2);
                }
                eprintln!("ningún fix propuesto sobre los bilinks que se pudieron mirar");
                std::process::exit(1);
            }

            // Collect state names for commit message summary
            let mut state_set: Vec<&str> = fixes.iter().map(|f| f.reason).collect();
            state_set.dedup();
            let states_label = state_set.join(" + ");

            // Print summary
            let max_state = fixes.iter().map(|f| f.reason.len()).max().unwrap_or(0);
            println!("Pending fixes ({}):", fixes.len());
            for f in &fixes {
                println!("  {:<width$}  {}…  link.{}  {}",
                    f.reason, f.short(), f.n,
                    f.description(),
                    width = max_state,
                );
            }

            if dry_run {
                // Un `--dry-run` con agujeros tampoco sale con 0: el resumen que acaba
                // de imprimir no cubre la capa entera, y el código de salida es lo único
                // que se lo dice a un script.
                if !unlooked.is_empty() { std::process::exit(1); }
                return Ok(());
            }

            // Confirm
            if !yes {
                eprint!("\nApply? [y/N] ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if input.trim().to_lowercase() != "y" {
                    eprintln!("abortado");
                    std::process::exit(1);
                }
            }

            // **Un commit por `link` repuntado, no por invocación.** Repuntar un
            // vínculo a otro fragmento es una decisión, y las decisiones se firman y
            // se auditan de a una. Lo fuerza además el mensaje: `apply <uuid>.<N>
            // <capture-nuevo>` nombra **un** endpoint, y uno que nombrara tres no
            // sería reproducible contra el árbol de un solo padre.
            //
            // La absorción la escribe el primer `seal`, y las N decisiones cuelgan
            // de ella.
            let mut applied: Vec<std::path::PathBuf> = Vec::new();
            let mut errors  = 0usize;

            for f in &fixes {
                // El id que el mensaje de la ref nombra. Para un fix de vecindario
                // no hay **uno**: son N, y el que identifica el acto es el fragmento
                // cuyo vecindario se repuntó.
                let capture = match &f.what {
                    bilinker::apply::Fix::Fragment { to, .. } => to.id(),
                    bilinker::apply::Fix::Neighbourhood { to, .. } => to.to_string(),
                };
                match bilinker::apply::apply_fix(&cwd, f) {
                    Ok(paths) => {
                        applied.extend(paths.clone());
                        let command = bilinker::refmsg::RefCommand::Apply {
                            uuid: f.uuid.clone(), n: f.n, capture,
                        };
                        // Antes del corte los bilinks viven en la rama, y
                        // commitearlos ahí también es por fix.
                        let sealed = match bilinker::bilink_ref::absorb_act(&cwd) {
                            Ok(_) => seal_apply(&cwd, &root, &paths, command, f),
                            Err(e) => Err(e),
                        };
                        if let Err(e) = sealed {
                            eprintln!("error al commitear {}.{}: {e}", f.short(), f.n);
                            errors += 1;
                        }
                    }
                    Err(e) => {
                        eprintln!("error  {}.{}: {e}", f.short(), f.n);
                        errors += 1;
                    }
                }
            }

            if applied.is_empty() {
                eprintln!("ningún fix aplicado");
                std::process::exit(if errors > 0 { 1 } else { 2 });
            }

            let n = fixes.len() - errors;
            println!("\nRepuntados {n} endpoint(s) ({states_label}). Los {n} quedan en RELOCATED.");
            println!("  Revisar con `bilinker get <uuid>.<N>` y aprobar con `bilinker accept --place`.");
            if errors > 0 {
                eprintln!("{errors} fix(es) fallaron — ejecutar 'bilinker check .' para ver el estado actual");
                std::process::exit(1);
            }
            // Los fixes salieron bien y aun así la corrida no cubrió la capa: lo que
            // quedó sin mirar no lo arregla haber aplicado los demás.
            if !unlooked.is_empty() { std::process::exit(1); }
        }

        Command::RestoreN1 { path, recursive, dry_run, from } => {
            let base = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                .unwrap_or_else(|| cwd.clone());
            let layers = if recursive {
                bilinker::index::layer_roots(&base)
            } else {
                vec![base.clone()]
            };

            let mut total = 0usize;
            let mut salteados = 0usize;
            let mut capas = 0usize;
            for layer in &layers {
                let r = bilinker::restore_n1::restore(layer, from.as_deref(), dry_run)?;
                if r.no_backup || (r.restored.is_empty() && r.skipped.is_empty()) { continue; }
                capas += 1;
                let rel = layer.strip_prefix(&base).unwrap_or(layer);
                println!("{}", if rel.as_os_str().is_empty() { Path::new(".") } else { rel }.display());
                println!("  restituidos  {}", r.restored.len());
                if !r.skipped.is_empty() {
                    // **Nombrados y no sólo contados.** Un endpoint que se queda en
                    // `declined` es limpio para `check`, así que esta salida es el
                    // único registro de que ahí había un contrato.
                    let mut por_motivo: std::collections::BTreeMap<String, Vec<&str>> =
                        Default::default();
                    for (at, why) in &r.skipped {
                        por_motivo.entry(why.to_string()).or_default().push(at);
                    }
                    for (why, ats) in &por_motivo {
                        println!("  salteados    {}   {why}", ats.len());
                        println!("    {}", ats.join("  "));
                    }
                }
                total += r.restored.len();
                salteados += r.skipped.len();

                // La restitución devuelve decisiones, así que cierra con un commit
                // sobre la ref como cualquier otra — uno por capa, porque trae un
                // conjunto de vuelta de una sola vez.
                if !dry_run && r.touched() {
                    // **La prosa lleva los salteados por uuid.** Un endpoint que se
                    // queda en `declined` no vuelve a aparecer en ningún inventario,
                    // así que este commit es el único registro de que ahí había un
                    // contrato y de que no se pudo devolver.
                    let mut prosa = format!("{} restituidos", r.restored.len());
                    if !r.skipped.is_empty() {
                        prosa.push_str(&format!(", {} salteados: {}", r.skipped.len(),
                            r.skipped.iter().map(|(at, _)| at.as_str())
                                .collect::<Vec<_>>().join(" ")));
                    }
                    seal_with(layer, bilinker::refmsg::RefCommand::RestoreN1, Some(prosa))?;
                }
            }
            let verbo = if dry_run { "se restituirían" } else { "restituidos" };
            println!("\n{total} {verbo}, {salteados} salteados, en {capas} capa(s)");
        }

        Command::Migrate { path, recursive, dry_run, cut, rollback } => {
            let base = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                .unwrap_or_else(|| cwd.clone());
            let layers = if recursive {
                bilinker::index::layer_roots(&base)
            } else {
                vec![base.clone()]
            };

            // Las carpetas transitorias nunca se commitean.
            for layer in &layers {
                bilink_migrate::cut::exclude_in(&accreta_migrate::repo_root_of(layer))?;
            }

            // **Cuál migración se corta lo dice `.bilink/version`.** Con una sola el
            // corte podía estar cableado; con dos hay que elegir, y elegir por el
            // ledger falla en una capa que **nació** en un formato y nunca migró.
            let cortes = bilink_migrate::cut::cuts_for(&layers);

            if rollback {
                for (layer, m) in &cortes {
                    bilink_migrate::cut::rollback_of(layer, m.backup_dir)?;
                    println!("  restaurado  {}", layer.display());
                }
                // El ledger vuelve atrás con los archivos: si quedara escrito, el
                // repo diría estar migrado con el formato viejo en disco.
                let repos: std::collections::BTreeSet<PathBuf> =
                    layers.iter().map(|l| accreta_migrate::repo_root_of(l)).collect();
                for repo in &repos {
                    accreta_migrate::forget(repo, &bilink_migrate::all())?;
                }
                eprintln!("\ncorte deshecho en {} capa(s).", layers.len());
                return Ok(());
            }

            if cut {
                if dry_run {
                    anyhow::bail!("--cut y --dry-run se excluyen: el corte escribe");
                }
                let mut cuts = Vec::new();
                // Se planifican **todas** antes de mover ninguna: si una capa no
                // verifica, no se corta nada. Un corte a medias deja el repo con
                // dos formatos y ningún binario que entienda los dos.
                for (layer, m) in &cortes {
                    match bilink_migrate::cut::plan_cut_of(layer, m) {
                        Ok(c) => cuts.push(c),
                        Err(e) => anyhow::bail!("{layer:?}: {e}\n\nNo se cortó ninguna capa."),
                    }
                }
                for c in &cuts {
                    println!("  {}  {} bilink(s), {} capture(s)",
                             c.layer.display(), c.bilinks, c.captures);
                }
                for c in &cuts {
                    bilink_migrate::cut::execute(c)?;
                    // La cache se siembra después de mover: es de la herramienta, no
                    // del formato, así que la migración la devuelve y la escribe quien
                    // la entiende.
                    if !c.commits.is_empty() {
                        let mut cache = bilinker::cache::Cache::load(&c.layer);
                        for (uuid, n, commit) in &c.commits {
                            cache.set_commit(uuid, *n, commit);
                        }
                        cache.save(&c.layer)?;
                    }
                }
                // El ledger va acá: el estado recién ahora es verdadero.
                let written = accreta_migrate::record(&layers, &bilink_migrate::all())?;
                println!();
                for l in &written { println!("ledger: {}", l.display()); }
                eprintln!("\ncorte hecho en {} capa(s). Lo anterior queda en el backup de cada una.", cuts.len());
                eprintln!("Revisar con `bilinker check .` y commitear.");
                return Ok(());
            }

            let report = accreta_migrate::generate(&layers, &bilink_migrate::all(), dry_run)?;

            for id in &report.skipped {
                eprintln!("ya aplicada: {id}");
            }
            if report.is_noop() {
                eprintln!("nada que migrar ({} capa(s) revisada(s))", layers.len());
                return Ok(());
            }
            let mut last_repo: Option<&std::path::Path> = None;
            for a in &report.applied {
                if last_repo != Some(a.repo.as_path()) {
                    println!("\nrepo {}", a.repo.display());
                    last_repo = Some(a.repo.as_path());
                }
                println!("  {}{}", a.id, if dry_run { "  [dry-run]" } else { "" });
                for note in &a.notes {
                    println!("    {note}");
                }
                println!("    {} archivo(s) afectado(s)", a.changed.len());
            }
            if dry_run {
                eprintln!("\ndry-run: no se escribió nada");
            } else {
                eprintln!("\ngenerado. Revisar, y cortar con `bilinker migrate --cut`.");
                eprintln!("El ledger se escribe en el corte, no ahora.");
            }
        }

        Command::Index { sub } => match sub {
            IndexCommand::Build { path, recursive } => {
                let root = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                    .unwrap_or_else(|| cwd.clone());
                let layers = if recursive {
                    bilinker::index::layer_roots(&root)
                } else {
                    vec![root]
                };
                for layer in layers {
                    match bilinker::index::build(&layer) {
                        Ok(0) => {}
                        Ok(n) => {
                            let rel = layer.strip_prefix(&cwd).unwrap_or(&layer);
                            println!("index: {}/.bilink/.index  ({n} entries)", rel.display());
                        }
                        Err(e) => eprintln!("error building index for {}: {e}", layer.display()),
                    }
                }
            }

            IndexCommand::Status { path, recursive } => {
                let root = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                    .unwrap_or_else(|| cwd.clone());
                let layers = if recursive {
                    bilinker::index::layer_roots(&root)
                } else {
                    vec![root]
                };
                let mut any_problem = false;
                for layer in layers {
                    let rel = layer.strip_prefix(&cwd).unwrap_or(&layer);
                    match bilinker::index::status(&layer)? {
                        bilinker::index::IndexStatus::Ok =>
                            println!("{}/.bilink/.index  OK", rel.display()),
                        bilinker::index::IndexStatus::Stale { stale_count } => {
                            println!("{}/.bilink/.index  STALE  ({stale_count} bilink(s) newer)", rel.display());
                            any_problem = true;
                        }
                        bilinker::index::IndexStatus::Missing => {
                            println!("{}/.bilink/.index  MISSING", rel.display());
                            any_problem = true;
                        }
                    }
                }
                if any_problem { std::process::exit(1); }
            }
        },

        Command::Accept { target, place, content, no_n1, force } => {
            // Dispatch: uuid.N  |  uuid (both endpoints)  |  path / "."
            let is_uuid_n = (target.ends_with(".0") || target.ends_with(".1"))
                && target[..target.len()-2].chars().all(|c| c.is_ascii_hexdigit() || c == '-');
            let is_path = target == "." || target.contains('/') || target.contains('\\')
                || std::path::Path::new(&target).exists();

            // Qué dimensiones aprueba. Sin flags, las dos.
            let what = bilinker::accept::What {
                no_n1, force,
                ..match (place, content) {
                    (true, false) => bilinker::accept::What::place_only(),
                    (false, true) => bilinker::accept::What::content_only(),
                    _             => bilinker::accept::What::default(),
                }
            };

            // **Un commit por aceptación, no por invocación.** Cada endpoint aprobado
            // cierra con el suyo, y la absorción que los precede a todos la escribe
            // el primer `seal`: la granularidad sigue al objeto, así que un
            // `accept .` de veinte endpoints deja veinte decisiones auditables una
            // por una en vez de una que las disimula a todas.
            let accept_one = |uuid: &str, n: u8| -> anyhow::Result<()> {
                let r = bilinker::accept::accept(&cwd, uuid, n, what, Some(&lspd_neighbours::Lspd))?;
                print_accept_result(&r);
                if !r.wrote {
                    return Ok(());
                }
                seal(&cwd, bilinker::refmsg::RefCommand::Accept {
                    place: what.place, content: what.content, uuid: r.uuid.clone(), n: r.n,
                })
            };

            if is_uuid_n {
                // Un endpoint
                let (uuid, n) = parse_accept_target(&target)?;
                accept_one(&uuid, n)?;
            } else if is_path {
                // Bulk: all PENDING under path filter
                let filter = if target == "." { None } else { Some(target.trim_end_matches('/')) };
                let _ = filter;
                let targets = bilinker::accept::pending(&cwd);
                if targets.is_empty() {
                    eprintln!("nothing to accept");
                } else {
                    let mut count = 0;
                    for (uuid, n) in &targets {
                        match accept_one(uuid, *n) {
                            Ok(())  => count += 1,
                            Err(e) => eprintln!("warn  {}.{n}: {e}", &uuid[..8.min(uuid.len())]),
                        }
                    }
                    eprintln!("accepted {count} endpoint(s)");
                }
            } else {
                // UUID prefix: accept both endpoints
                let mut count = 0;
                for n in [0u8, 1u8] {
                    match accept_one(&target, n) {
                        Ok(())  => count += 1,
                        Err(e) => eprintln!("warn .{n}: {e}"),
                    }
                }
                if count > 0 {
                    eprintln!("note: adjacent node will detect CHAIN_DIRTY on next check");
                }
            }
        }

        Command::Remove { uuid } => {
            // `find_bilink_path` recibe la **capa** y le agrega `.bilink` sola.
            let path = bilinker::accept::find_bilink_path(&cwd, &uuid)?;
            std::fs::remove_file(&path)?;
            let rel = path.strip_prefix(&cwd).unwrap_or(&path);
            eprintln!("removed: {}", rel.display());
            eprintln!("note: nodos adyacentes detectarán BROKEN en el próximo check");
        }


        Command::Graph { selector, depth, format, recursive } => {
            let root = project_root(&cwd)?;
            cmd_graph(&root, &cwd, &selector, &format, depth, recursive)?;
        }

        Command::Init { dry_run } => {
            print_init(bilinker::init::init(&cwd, dry_run)?, dry_run);
        }

        Command::Sync { dry_run } => {
            print_sync(bilinker::sync::sync(&cwd, dry_run)?, dry_run);
        }

        Command::VerifyRef { range, signers, stdin } => {
            use bilinker::verify;
            let signers = signers.as_deref();

            let objetivos: Vec<(String, Option<String>, String)> = if stdin {
                let mut buf = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
                verify::parse_stdin(&buf)
                    .into_iter()
                    .map(|(o, n, r)| (r, Some(o), n))
                    .collect()
            } else {
                vec![verify::target(&cwd, range.as_deref())?]
            };

            let mut rechazos = 0usize;
            for (refname, old, new) in objetivos {
                let r = verify::verify(&cwd, &refname, old.as_deref(), &new, signers)?;
                print_verify(&r);
                rechazos += r.rejected();
            }
            if rechazos > 0 {
                std::process::exit(1);
            }
        }

        Command::Push { branch, remote } => {
            let r = match bilinker::push::push(&cwd, branch.as_deref(), remote.as_deref()) {
                Ok(r) => r,
                Err(e) => {
                    // **Un non-fast-forward tiene dos causas**, y confundirlas manda
                    // a mirar un incidente donde no hubo ninguno. `git merge-base`
                    // las separa, así que no hace falta adivinar.
                    let elegido = bilinker::push::pick_remote(
                        &bilinker::bilink_ref::Repo::open(&cwd)?, remote.as_deref());
                    if let Ok(rem) = elegido {
                        if let Ok(diag) = bilinker::pull::diagnose_rejection(&cwd, &rem) {
                            eprintln!("error: {diag}");
                            std::process::exit(1);
                        }
                    }
                    return Err(e);
                }
            };
            if r.moved {
                println!("publicado: refs/bilink/{} @ {} → {}",
                         r.branch, short(&r.tip), r.remote);
            } else {
                println!("refs/bilink/{} ya estaba en {} @ {}",
                         r.branch, r.remote, short(&r.tip));
            }
        }

        Command::Track { branch, from } => {
            let r = bilinker::track::track(&cwd, &branch, from.as_deref())?;
            match (&r.inherited, &r.base) {
                (Some(m), Some(p)) => println!(
                    "hereda:  {} sobre {}\ncommit:  refs/bilink/{} @ {}\nárbol:   {} archivo(s)",
                    short(m), short(p), r.branch, short(&r.sha), r.files
                ),
                _ => println!(
                    "ningún commit de refs/bilink/* califica: la ref nace desde cero.\n\
                     commit:  refs/bilink/{} @ {}\nárbol:   {} archivo(s)",
                    r.branch, short(&r.sha), r.files
                ),
            }
        }

        Command::Relayer { layer, dry_run } => {
            let layer = layer.trim_end_matches('/').to_string();
            let r = bilinker::relayer::relayer(&cwd, &layer, dry_run)?;
            println!("{}: {} capture(s) reacuñados, {} bilink(s) movidos, \
                      {} vecino(s) con el id actualizado",
                     r.layer, r.captures, r.bilinks, r.neighbours);
            if dry_run {
                println!("\ndry-run: no se escribió nada");
            } else {
                seal(&cwd, bilinker::refmsg::RefCommand::Relayer { layer: r.layer })?;
            }
        }

        Command::History { target, format } => {
            let (uuid, n) = bilinker::history::parse_target(&target)?;
            let h = bilinker::history::history(&cwd, &uuid, n)?;
            if format.as_deref() == Some("json") {
                println!("{}", serde_json::to_string_pretty(&h)?);
            } else {
                print_history(&h);
            }
        }

        Command::Pull { remote, dry_run } => {
            let r = bilinker::pull::pull(&cwd, remote.as_deref(), dry_run)?;
            print_pull(&r, dry_run);
            if r.conflicts() > 0 { std::process::exit(1); }
        }

        Command::Adopt { branch, dry_run } => {
            let r = bilinker::adopt::adopt(&cwd, &branch, dry_run)?;
            print_adopt(&r, dry_run);
            if r.conflicts() > 0 { std::process::exit(1); }
        }

        Command::Fetch { alias } => {
            let aliases = match alias {
                Some(a) => vec![a],
                None => bilinker::frontier::declared_aliases(&cwd),
            };
            if aliases.is_empty() {
                eprintln!("no hay ningún proveedor declarado (.bilink/.{{alias}}.toml)");
            }
            for a in aliases {
                let r = bilinker::frontier::fetch(&cwd, &a)?;
                println!("{}: refs/bilink/{} · {} archivo(s) en el sparse",
                         r.alias, r.branch, r.files);
            }
        }

        Command::Abstracts { alias, lines } => {
            let (items, label) = match &alias {
                Some(a) => (bilinker::frontier::abstracts(&cwd, a)?, a.clone()),
                None    => (bilinker::frontier::published(&cwd)?, "esta capa".into()),
            };
            print_abstracts(&items, &label, lines, alias.is_some());
        }

        Command::Status { path, porcelain } => {
            let layer = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                .unwrap_or_else(|| cwd.clone());
            if porcelain {
                for (change, file) in bilinker::review::status(&layer)? {
                    println!("{} {file}", change.letter());
                }
            } else {
                print_status(&layer)?;
            }
        }

        Command::Log { branch, not } => {
            let lines = bilinker::review::log(&cwd, branch.as_deref(), not.as_deref())?;
            if lines.is_empty() {
                eprintln!("ningún acto registrado en la ref");
            }
            for line in lines {
                println!("{line}");
            }
        }

        Command::Diff { against } => {
            print!("{}", bilinker::review::diff(&cwd, against.as_deref())?);
        }

        Command::Chain { sub } => match sub {
            ChainCommand::New { tip, mid, kind, name0, name1, from_repo, as0, as1, list_modes, dry_run, yes } => {
                if list_modes {
                    for g in bilinker::capture::generators() {
                        println!("  {:<20} {}", g.name(), g.describe());
                    }
                    return Ok(());
                }
                let modes = [
                    as0.as_deref().map(bilinker::capture::generator_named).transpose()?,
                    as1.as_deref().map(bilinker::capture::generator_named).transpose()?,
                ];
                let root = project_root(&cwd)?;

                // Con `--from-repo`, el tip del proveedor lo aporta el flag: quien
                // consume escribe **un solo** `--tip`, el suyo.
                let (from_repo_uuid, mut plans, remote) = match &from_repo {
                    Some(spec) => {
                        if tip.len() != 1 {
                            anyhow::bail!(
                                "con --from-repo va un solo --tip: el del lado local. \
                                 El otro es el repo del proveedor."
                            );
                        }
                        let (alias, uuid) = spec.split_once(':').ok_or_else(|| {
                            anyhow::anyhow!("--from-repo se escribe `<alias>:<uuid>`")
                        })?;
                        let plan = plan_tip(&root, &tip[0], modes[0].as_deref())?;
                        let remote = (
                            plan.layer_fs.clone(),
                            bilink_format::LinkEndpoint::Repo(alias.to_string()),
                        );
                        (Some(uuid.to_string()), vec![plan], Some(remote))
                    }
                    None => {
                        if tip.len() != 2 {
                            anyhow::bail!("chain new requires exactly 2 --tip REF arguments");
                        }
                        (None, vec![plan_tip(&root, &tip[0], modes[0].as_deref())?,
                                    plan_tip(&root, &tip[1], modes[1].as_deref())?], None)
                    }
                };

                // La vista previa, y la oportunidad de corregirla. Un capture es
                // opaco después de escrito: acá todavía se puede no escribirlo.
                if !confirm_tips(&mut plans, dry_run, yes)? {
                    eprintln!("no se escribió nada.");
                    return Ok(());
                }
                if dry_run { return Ok(()); }

                let mut tips: Vec<(PathBuf, bilink_format::LinkEndpoint)> = Vec::new();
                if let Some(r) = remote { tips.push(r); }
                for plan in &plans {
                    tips.push((plan.layer_fs.clone(), plan.write()?));
                }

                let mids: Vec<PathBuf> = mid.iter().map(PathBuf::from).collect();

                // `kind` y `name` son declaración, y todo archivo de bilinker sale
                // de un comando: sin estos flags la única forma de poblarlos sería
                // abrir el YAML, que es lo que el formato no le pide a nadie.
                //
                // El `as` no se pide con un flag propio: es el nombre que ya tomó
                // `--as.N`, anotado donde quede legible después. Y se acomoda a
                // dónde cayó cada tip: con `--from-repo` el `--tip` que se escribió
                // es uno solo y termina en la posición 1, porque la 0 la ocupa el
                // repo del proveedor — que no captura nada y por eso no lleva `as`.
                let as_by_tip = match from_repo {
                    Some(_) => [None, as0],
                    None    => [as0, as1],
                };
                let decl = bilinker::chain::Declaration {
                    kind, name: [name0, name1], r#as: as_by_tip, uuid: from_repo_uuid,
                };
                let result = bilinker::chain::chain_new(&cwd, &tips, &mids, &decl)?;

                println!("Created chain: {}", result.uuid);
                println!();
                for f in &result.files {
                    let rel = f.strip_prefix(&cwd).unwrap_or(f);
                    println!("  {}", rel.display());
                }
                println!();
                eprintln!("Run 'bilinker check .' to populate cache.");
            }

            ChainCommand::Status { uuid } => {
                let root = project_root(&cwd)?;
                print_chain_status(&root, &uuid)?;
            }

            ChainCommand::List { patron, as_, link, state, under } => {
                let root = project_root(&cwd)?;
                list_chains(&root, &ChainFilter { patron, as_, link, state, under })?;
            }
        },
    }
    Ok(())
}

/// La salida de `init`, que dice qué escribió y qué no.
fn print_init(r: bilinker::init::InitResult, dry_run: bool) {
    use bilinker::init::Outcome;

    let line = |label: &str, added: bool, what: String| {
        println!("{label}: {}", if added { what } else { "ya estaba".into() });
    };
    line("exclude", !r.excluded.is_empty(), format!("+ {}", r.excluded.join("  + ")));
    line("refspec", !r.refspec.is_empty(),
         format!("+ {}  ({})", bilinker::config::REFSPEC, r.refspec.join(", ")));
    if r.fetched.is_some() {
        println!("fetch:   refs/bilink/* traídas");
    }

    match r.outcome {
        Outcome::Materialized { commit, files } => println!(
            "árbol:   .bilink/ materializado desde refs/bilink/{} @ {}\n         {files} archivo(s)",
            r.branch.unwrap_or_default(), short(&commit)
        ),
        Outcome::AlreadyCurrent { commit } => println!(
            "árbol:   al día · refs/bilink/{} @ {}",
            r.branch.unwrap_or_default(), short(&commit)
        ),
        // Es lo esperado en el paso 3 del corte: ahí el `.bilink/` recién puesto
        // todavía no está en la ref, y materializar lo borraría.
        Outcome::SkippedNoProvenance => println!(
            "\n.bilink/ presente sin head: no se materializa nada.\n  \
             Es lo esperado en el paso 3 del corte 005; en un clon fresco, revisar\n  \
             de dónde salió antes de seguir."
        ),
        Outcome::NoRef(branch) => println!(
            "\nrefs/bilink/{branch} no existe.\n  \
             Correr `bilinker track {branch}` para crearla."
        ),
        Outcome::Detached => println!("\nHEAD desacoplado: no se materializa nada."),
    }
    if dry_run {
        println!("\ndry-run: no se escribió nada");
    }
}

/// La salida de `sync`. El diff vacío se dice, porque es lo que lo identifica como
/// el acto que no registra ninguna decisión.
fn print_sync(r: bilinker::sync::SyncResult, dry_run: bool) {
    match (&r.absorbed, r.commits) {
        (None, 0) => println!(
            "refs/bilink/{} ya absorbió {} @ {} — nada que hacer",
            r.branch, r.branch,
            r.at.as_deref().map(short).unwrap_or("—")
        ),
        (Some(tip), _) => {
            println!("absorbe:  {}  → {}", r.branch, short(tip));
            if dry_run {
                println!("disyunción: ok — el árbol del commit no trae .bilink/");
            } else {
                println!("commit:   refs/bilink/{}  {} → {}", r.branch, short(&r.from), short(&r.to));
                println!("diff:     vacío — ninguna decisión registrada");
            }
        }
        (None, _) => println!("commit:   refs/bilink/{}  {} → {}", r.branch, short(&r.from), short(&r.to)),
    }
    if dry_run {
        println!("\ndry-run: no se escribió nada");
    }
}

/// El catálogo de abstracciones. Muestra **el código**, que es lo que hace falta
/// para decidir de cuál colgarse — una lista de uuids no alcanza para elegir.
fn print_abstracts(
    items: &[bilinker::frontier::Abstraction], label: &str, lines: usize, remote: bool,
) {
    if items.is_empty() {
        eprintln!("{label} no publica ninguna abstracción");
        return;
    }
    println!("{label} · {} abstracción(es)\n", items.len());

    for a in items {
        let id = &a.uuid[..8.min(a.uuid.len())];
        let marca = if a.consumed { "   ← ya lo consumís" } else { "" };
        match &a.name {
            Some(n) => println!("  {id}  {}  ({}){marca}", a.file, n),
            None    => println!("  {id}  {}{marca}", a.file),
        }

        match &a.text {
            Some(t) => {
                let total = t.lines().count();
                let show: Vec<&str> = if lines == 0 { t.lines().collect() }
                                      else { t.lines().take(lines).collect() };
                for l in &show {
                    println!("            {l}");
                }
                if show.len() < total {
                    println!("            … {} línea(s) más", total - show.len());
                }
            }
            // El capture no resuelve contra esa versión: se dice, no se inventa.
            None => println!("            (el fragmento no se pudo resolver)"),
        }
        println!();
    }

    if remote {
        println!("Para colgarse de una: bilinker chain new --from-repo '{label}:<uuid>' --tip <tu fragmento>");
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// Cierra una decisión con sus commits sobre la ref: la absorción si falta, y la
/// decisión.
///
/// **Son dos commits porque son dos cosas.** Absorber es precondición de todo commit
/// sobre la ref y se cumple en un commit propio, inmediatamente anterior; nunca en el
/// mismo. Llamar acá N veces seguidas absorbe **una** sola —la segunda encuentra el
/// tip ya absorbido— así que las N decisiones de un `accept .` cuelgan todas del
/// mismo merge sin que nadie lleve la cuenta.
///
/// No hace nada en un repo que todavía no cortó a la ref, donde los bilinks viven en
/// la rama del proyecto y commitearlos es de quien trabaja.
fn seal(cwd: &Path, command: bilinker::refmsg::RefCommand) -> anyhow::Result<()> {
    seal_with(cwd, command, None)
}

/// Ídem, con prosa. La lleva quien commitea **un conjunto** de decisiones de una vez:
/// sin endpoint en la primera línea, lo que dice qué se decidió es la prosa.
fn seal_with(
    cwd: &Path,
    command: bilinker::refmsg::RefCommand,
    prose: Option<String>,
) -> anyhow::Result<()> {
    if let Some(a) = bilinker::bilink_ref::absorb_act(cwd)? {
        eprintln!("commit:  refs/bilink/… @ {}  (absorbe {})", short(&a.sha),
                  short(a.absorbed.as_deref().unwrap_or("?")));
    }
    let mut message = bilinker::refmsg::RefMessage::new(command).with_invocation(invocation());
    if let Some(p) = prose { message = message.with_prose(p); }
    match bilinker::bilink_ref::decide_act(cwd, &message)? {
        Some(c) if c.wrote => eprintln!("commit:  refs/bilink/… @ {}", short(&c.sha)),
        _ => {}
    }
    Ok(())
}

/// El commit de decisión de **un** fix de `apply`, sobre la ref o sobre la rama.
///
/// En un repo que todavía no cortó a la ref los bilinks viven en la rama del
/// proyecto, y ahí el commit es un commit común — pero sigue siendo uno por fix, que
/// es lo que hace que las dos historias se lean igual.
fn seal_apply(
    cwd: &Path,
    root: &Path,
    paths: &[std::path::PathBuf],
    command: bilinker::refmsg::RefCommand,
    f: &bilinker::apply::PendingFix,
) -> anyhow::Result<()> {
    let message = bilinker::refmsg::RefMessage::new(command)
        .with_prose(format!("{} {}", f.reason, f.description()))
        .with_invocation(invocation());

    match bilinker::bilink_ref::decide_act(cwd, &message)? {
        Some(c) if c.wrote => {
            eprintln!("commit:  refs/bilink/… @ {}", short(&c.sha));
            Ok(())
        }
        Some(_) => Ok(()),
        None => git_commit(root, paths, &message.render()).map(|_| ()),
    }
}

/// Lo que la persona tipeó, para el trailer `Invocation:`.
///
/// **Es auditoría y no verificación**: un `accept .` de veinte endpoints escribe
/// veinte commits, y cada uno lleva su propio comando canónico. Esto guarda el
/// `accept .` que los produjo, que es lo único que la primera línea ya no dice.
fn invocation() -> Vec<String> {
    std::iter::once("bilinker".to_string())
        .chain(std::env::args().skip(1))
        .collect()
}

/// Las tres filas de `adopt`, agrupadas. Son las únicas posibles: `accepted` son
/// campos con nombre, por endpoint, así que el merge a tres puntas los compara de a
/// uno.
fn print_adopt(r: &bilinker::adopt::AdoptResult, dry_run: bool) {
    use bilinker::adopt::Row;

    if r.changes.is_empty() && r.commits == 0 {
        println!("refs/bilink/{} no avanzó desde la base — nada que adoptar", r.neighbour);
        return;
    }

    match &r.base {
        Some(b) => println!("base {} · {} de {}\n", short(b), plural(r.changes.len()), r.neighbour),
        None => println!("sin base de merge con {} — toda diferencia es conflicto\n", r.neighbour),
    }

    let mut last = None;
    for (row, label) in [(Row::Clean, "entra limpio"), (Row::Converged, "ya coincidía"),
                         (Row::Conflict, "conflicto   ")] {
        for c in r.changes.iter().filter(|c| c.row == row) {
            let head = if last == Some(row) { "            " } else { label };
            last = Some(row);
            match row {
                Row::Conflict => println!(
                    "{head}     {}.{}   {}    {} {}  ·  acá {}",
                    &c.uuid[..8.min(c.uuid.len())], c.n, c.dimension, r.neighbour,
                    trunc(c.theirs.as_deref()), trunc(c.mine.as_deref())
                ),
                Row::Converged => println!(
                    "{head}     {}.{}   {}    — mismo valor de los dos lados",
                    &c.uuid[..8.min(c.uuid.len())], c.n, c.dimension
                ),
                Row::Clean => println!(
                    "{head}     {}.{}   {}",
                    &c.uuid[..8.min(c.uuid.len())], c.n, c.dimension
                ),
            }
        }
    }

    let conflicts = r.conflicts();
    if conflicts > 0 {
        println!(
            "\n{conflicts} conflicto(s): no se escribió nada.\n  \
             Revisar con `bilinker get <uuid>.<N> --diff` y decidir con `bilinker accept`."
        );
        return;
    }
    if dry_run {
        println!("\ndry-run: no se escribió nada");
        return;
    }
    if let Some(a) = &r.absorbed {
        println!("\nabsorbe:  {} → {}", r.branch, short(a));
    }
    if r.commits > 0 {
        println!("commit:   refs/bilink/{}  +{} endpoint(s)", r.branch, r.adopted());
    }
}

fn plural(n: usize) -> String {
    if n == 1 { "1 diferencia".into() } else { format!("{n} diferencias") }
}

fn trunc(s: Option<&str>) -> String {
    match s {
        Some(v) if v.len() > 8 => format!("{}…", &v[..8]),
        Some(v) => v.to_string(),
        None => "—".into(),
    }
}

/// El estado de una cadena, recorriendo sus nodos.
///
/// Los estados salen de la cache de cada capa: no están en los archivos. Con la
/// cache fría se dice, en vez de inventar un OK.
fn print_chain_status(root: &Path, uuid: &str) -> anyhow::Result<()> {
    let mut nodes: Vec<(PathBuf, bilink_format::BiLink)> = Vec::new();
    for (layer, _) in layers_with(root, uuid) {
        let path = bilink_format::BiLink::path_in(&layer, uuid);
        if let Ok(bl) = bilink_format::BiLink::load(&path) {
            nodes.push((layer, bl));
        }
    }
    if nodes.is_empty() {
        anyhow::bail!("no existe la cadena '{uuid}'");
    }

    println!("Cadena: {uuid}  [{}]", chain_overall_state(root, uuid, &nodes));
    println!();
    for (layer, bl) in &nodes {
        let cache = bilinker::cache::Cache::load(layer);
        let label = layer.strip_prefix(root).ok()
            .map(|p| if p.as_os_str().is_empty() { ".".into() } else { p.display().to_string() })
            .unwrap_or_else(|| layer.display().to_string());

        println!("  {label}/  ({}, {})", state_label(&cache, uuid, 0), state_label(&cache, uuid, 1));
        println!("    endpoint.0  {}", bl.endpoint.zero.link);
        println!("    endpoint.1  {}", bl.endpoint.one.link);
    }
    Ok(())
}

fn state_label(cache: &bilinker::cache::Cache, uuid: &str, n: u8) -> String {
    cache.endpoint_state(uuid, n).map(|s| s.to_string()).unwrap_or_else(|| "—".into())
}

/// Las capas que tienen un bilink con este uuid.
fn layers_with(root: &Path, uuid: &str) -> Vec<(PathBuf, PathBuf)> {
    bilinker::index::layer_roots(root).into_iter()
        .map(|l| (l.clone(), bilink_format::BiLink::path_in(&l, uuid)))
        .filter(|(_, p)| p.exists())
        .collect()
}

fn list_chains(root: &Path, filtro: &ChainFilter) -> anyhow::Result<()> {
    use std::collections::BTreeMap;
    let mut chains: BTreeMap<String, usize> = BTreeMap::new();

    for layer in bilinker::index::layer_roots(root) {
        for path in bilink_format::bilink::bilink_files(&layer.join(".bilink")) {
            if let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) {
                *chains.entry(uuid.to_string()).or_default() += 1;
            }
        }
    }
    if chains.is_empty() {
        println!("(no hay cadenas)");
        return Ok(());
    }
    let total = chains.len();
    let mut mostradas = 0usize;
    for (uuid, n) in chains {
        let nodes: Vec<(PathBuf, bilink_format::BiLink)> = layers_with(root, &uuid).into_iter()
            .filter_map(|(l, p)| bilink_format::BiLink::load(&p).ok().map(|bl| (l, bl)))
            .collect();
        let alias = alias_de_cadena(&uuid, &nodes);
        let estado = chain_overall_state(root, &uuid, &nodes);
        if !filtro.pasa(alias.as_deref(), estado, &nodes) { continue }
        mostradas += 1;
        // **El nombre antes que el conteo.** Con 98 cadenas, `1 nodo(s)` repetido no
        // distingue nada; el alias sí. Sin alias se cae al conteo, que es lo que hay
        // para todo lo escrito antes de que `as` existiera.
        let como = alias.unwrap_or_else(|| format!("{n} nodo(s)"));
        println!("{}  [{estado}]  {como}", &uuid[..8.min(uuid.len())]);
    }
    // **Cuántas de cuántas**, y sólo con filtro puesto: es lo que dice si el filtro
    // acertó o si dejó afuera lo que se buscaba.
    if !filtro.vacio() {
        println!();
        println!("{mostradas} de {total}");
        if mostradas == 0 {
            println!("(ningún filtro matcheó — `chain list` sin argumentos las lista todas)");
        }
    }
    Ok(())
}

/// Cómo se llama la cadena: el alias del primer extremo que sepa nombrarse.
///
/// Los dos tips rara vez se nombran igual —de un lado hay una sección de markdown y
/// del otro un endpoint— y el que sabe nombrar es el que tiene generador. Con los dos
/// nombrados gana el primero, que es un desempate arbitrario y no importa: una cadena
/// tiene un nombre, no dos.
fn alias_de_cadena(uuid: &str, nodes: &[(PathBuf, bilink_format::BiLink)]) -> Option<String> {
    for (layer, _) in nodes {
        let cache = bilinker::cache::Cache::load(layer);
        for n in [0u8, 1u8] {
            if let Some(a) = cache.alias(uuid, n) { return Some(a.to_string()) }
        }
    }
    None
}

/// El peor estado de la cadena.
fn chain_overall_state(_root: &Path, uuid: &str, nodes: &[(PathBuf, bilink_format::BiLink)]) -> &'static str {
    use bilinker::state::EndpointState::*;
    let mut seen = Vec::new();
    for (layer, _) in nodes {
        let cache = bilinker::cache::Cache::load(layer);
        for n in [0u8, 1u8] {
            if let Some(s) = cache.endpoint_state(uuid, n) { seen.push(s); }
        }
    }
    if seen.is_empty() { return "—"; }
    if seen.iter().any(|s| matches!(s, Altered | Unresolved | Broken)) { return "BROKEN"; }
    if seen.iter().any(|s| matches!(s, ChainDirty)) { return "DIRTY"; }
    if seen.iter().any(|s| !s.is_ok()) { return "PENDIENTE"; }
    "OK"
}

fn watch(root: &Path) -> anyhow::Result<()> {
    use notify::{EventKind, RecursiveMode, Watcher, recommended_watcher};
    use bilink_format::bilink::bilink_files;
    
    use std::sync::mpsc;

    eprintln!("watching {}  (Ctrl-C to stop)", root.display());

    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = recommended_watcher(tx)?;
    watcher.watch(root, RecursiveMode::Recursive)?;

    for res in rx {
        let event = match res {
            Ok(e)  => e,
            Err(e) => { eprintln!("watch error: {e}"); continue; }
        };

        if !matches!(event.kind, EventKind::Modify(_)) { continue; }

        'paths: for path in &event.paths {
            if path.components().any(|c| c.as_os_str() == ".bilink") { continue; }
            if !path.is_file() { continue; }

            let rel = match path.strip_prefix(root) {
                Ok(r)  => r.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            let mut chains: Vec<String> = Vec::new();
            for entry in bilink_files(&root.join(".bilink")) {
                let Ok(bl) = bilink_format::BiLink::load(&entry) else { continue };
                let Some(uuid) = entry.file_stem().and_then(|s| s.to_str()) else { continue };

                let references_file = (0..2u8).any(|n| {
                    match bilinker::capture::capture_of(root, &bl.endpoint.get(n).link) {
                        Ok(Some(cap)) => rel.contains(&cap.file) || cap.file.contains(&rel),
                        _ => false,
                    }
                });
                if references_file { chains.push(uuid.to_string()); }
            }

            if !chains.is_empty() {
                for chain in &chains {
                    println!("ALTERED  {rel}  chain {chain}");
                }
            }

            break 'paths;
        }
    }
    Ok(())
}

fn print_accept_result(r: &bilinker::accept::AcceptResult) {
    let commit = match &r.commit {
        Some(c) => c[..12.min(c.len())].to_string(),
        None    => "(sin commit)".to_string(),
    };
    let agree: Vec<&str> = r.agree.iter().map(String::as_str).collect();
    println!("  {}.{}  {}  {}  agree: {}",
             &r.uuid[..8.min(r.uuid.len())], r.n,
             &r.hash[..12.min(r.hash.len())], commit, agree.join(", "));
    // Aprobar dos veces lo mismo no dice nada nuevo, y se dice en vez de escribir un
    // commit vacío.
    if !r.wrote {
        println!("        ya estabas en el set y los valores no se movieron — nada que agregar");
    }
}

/// La historia de un bilink.
///
/// **Lo que no se sabe se dice.** Un acto anterior a la gramática no tiene comando
/// que leer, y ponerle uno derivado del texto libre sería fabricar precisión.
fn print_history(h: &bilinker::history::History) {
    println!("{}  {}", h.uuid, h.path);
    if !h.from_ref {
        println!("sin ref: la historia sale de la rama, y no incluye los actos que la \
                  ref registraría");
    }
    if h.deeds.is_empty() {
        println!("\nsin actos");
        return;
    }

    for d in &h.deeds {
        let comando = d.command.as_deref().unwrap_or("(anterior a la gramática)");
        println!("\n  {}  {:<8} {:<11}  {:<15}  {comando}",
                 short(&d.commit), d.author, &d.date[..10.min(d.date.len())], d.kind);
        if let Some(a) = &d.against {
            println!("           contra {}", short(a));
        }
        for c in &d.changes {
            println!("           .{}  {:<14} {} → {}", c.n, c.field,
                     abbrev(c.before.as_deref()), abbrev(c.after.as_deref()));
            for cap in &c.captures {
                // La query se aplana: es multilínea en el archivo, y acá cada acto
                // tiene que caber en su renglón para que el orden se lea.
                let q = cap.query.as_deref().map(one_line);
                println!("               {}  {}  {}", &cap.id[..8.min(cap.id.len())],
                         cap.file, q.as_deref().unwrap_or("(archivo entero)"));
            }
        }
    }
}

/// Una query en un renglón, recortada.
fn one_line(q: &str) -> String {
    let plano = q.split_whitespace().collect::<Vec<_>>().join(" ");
    if plano.len() > 60 { format!("{}…", &plano[..60]) } else { plano }
}

/// Un valor de la historia, acortado si es un hash. `—` para lo que no estaba.
fn abbrev(v: Option<&str>) -> String {
    match v {
        None => "—".to_string(),
        Some(s) if s.len() > 20 && s.chars().all(|c| c.is_ascii_hexdigit()) =>
            format!("{}…", &s[..8]),
        Some(s) if s.len() > 40 => format!("{}…", &s[..40]),
        Some(s) => s.to_string(),
    }
}

/// El informe de `pull`.
fn print_pull(r: &bilinker::pull::PullResult, dry_run: bool) {
    use bilinker::adopt::Row;

    if r.up_to_date {
        println!("refs/bilink/{} ya tiene lo de {} — nada que traer", r.branch, r.remote);
        return;
    }
    if r.fast_forward {
        println!("refs/bilink/{} avanzó a {} — no hubo nada que unir",
                 r.branch, short(r.sha.as_deref().unwrap_or("")));
        return;
    }

    match &r.base {
        Some(b) => println!("base {} · aceptaciones de {} en {}..\n",
                            short(b), r.remote, short(b)),
        None => println!("sin base de merge con {} — toda diferencia es conflicto\n", r.remote),
    }

    for (row, label) in [(Row::Clean, "entra limpio"), (Row::Converged, "ya coincidía"),
                         (Row::Conflict, "conflicto   ")] {
        for c in r.changes.iter().filter(|c| c.row == row) {
            println!("  {label}  {}.{}  {}", &c.uuid[..8.min(c.uuid.len())], c.n, c.dimension);
            if row == Row::Conflict {
                println!("                  acá:   {}", c.mine.as_deref().unwrap_or("—"));
                println!("                  allá:  {}", c.theirs.as_deref().unwrap_or("—"));
            }
        }
    }

    if r.conflicts() > 0 {
        let primero = r.changes.iter().find(|c| c.row == Row::Conflict).expect("hay uno");
        println!("\nno se escribió nada. Resolver aceptando uno de los dos: \
                  `bilinker accept {}.{}`",
                 &primero.uuid[..8.min(primero.uuid.len())], primero.n);
        return;
    }
    if dry_run {
        println!("\ndry-run: no se escribió nada");
        return;
    }
    match &r.sha {
        Some(sha) => println!("\ncommit:  refs/bilink/{} @ {}   ({} endpoint(s))",
                              r.branch, short(sha), r.brought()),
        None => println!("\nnada que traer"),
    }
}

/// El informe de `verify-ref`.
///
/// **Lo que no se verificó se dice.** Confundir "verifiqué y está bien" con "no
/// verifiqué" sería el peor resultado posible de una herramienta de verificación.
fn print_verify(r: &bilinker::verify::Report) {
    println!("{}  {} commit(s)\n", r.refname, r.verdicts.len());

    let rechazados = r.rejected();
    if rechazados == 0 {
        let previos = r.pre_grammar();
        let firmados = if r.signatures_checked { ", firmados" } else { "" };
        println!("  ✓  {:>3}  con la gramática{firmados}", r.verdicts.len() - previos);
        if previos > 0 {
            println!("  ·  {previos:>3}  anteriores a la gramática — forma no verificada");
        }
        if !r.signatures_checked {
            println!("\nsin allowlist: la firma no se verificó");
        }
        println!("\nok");
        return;
    }

    for v in r.verdicts.iter().filter(|v| !v.faults.is_empty()) {
        for f in &v.faults {
            println!("  ✗  {}  {f}", short(&v.commit));
        }
    }
    println!("\n{rechazados} de {} rechazados", r.verdicts.len());
}

/// El estado de la capa, agrupado por archivo.
///
/// Lee la cache; no resuelve nada. Con la cache fría **no hay estados** y se dice:
/// mostrar OK sin haber verificado sería peor que no mostrar nada.
fn print_status(layer: &Path) -> anyhow::Result<()> {
    use std::collections::BTreeMap;
    use bilink_format::{BiLink, Capture};

    let bilink_dir = layer.join(".bilink");
    if !bilink_dir.exists() {
        eprintln!("no hay .bilink/ en {}", layer.display());
        return Ok(());
    }

    let cache = bilinker::cache::Cache::load(layer);
    let mut cold = true;

    struct Row { file_name: String, uuid_short: String, s0: String, s1: String }
    let mut groups: BTreeMap<String, Vec<Row>> = BTreeMap::new();

    for path in bilink_format::bilink::bilink_files(&bilink_dir) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        // Se agrupa por el directorio del endpoint estructural.
        let file = (0..2u8)
            .filter_map(|n| bl.endpoint.get(n).link.capture_id())
            .filter_map(|id| Capture::load_in(layer, id).ok())
            .map(|c| c.file)
            .next();

        let (dir, file_name) = match file.as_ref() {
            Some(f) => {
                let p = std::path::Path::new(f);
                let dir = p.parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|| ".".into());
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| f.clone());
                (dir, name)
            }
            None => ("(layer)".into(), uuid.to_string()),
        };

        let label = |n: u8| match cache.endpoint_state(uuid, n) {
            Some(st) => { st.to_string() }
            None     => "—".to_string(),
        };
        let (s0, s1) = (label(0), label(1));
        if cache.endpoint_state(uuid, 0).is_some() { cold = false; }

        groups.entry(dir).or_default().push(Row {
            file_name, uuid_short: uuid[..8.min(uuid.len())].to_string(), s0, s1,
        });
    }

    if groups.is_empty() {
        println!("(no hay bilinks)");
        return Ok(());
    }

    for (dir, mut rows) in groups {
        println!("{dir}/");
        rows.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        let max_name = rows.iter().map(|r| r.file_name.len()).max().unwrap_or(0);
        let mut prev = String::new();
        for row in &rows {
            let label = if row.file_name != prev {
                format!("{:<width$}", row.file_name, width = max_name)
            } else {
                format!("{:<width$}", "", width = max_name)
            };
            println!("  {}  {}  ({}, {})", label, row.uuid_short, row.s0, row.s1);
            prev = row.file_name.clone();
        }
        println!();
    }

    if cold {
        eprintln!("sin estados: la cache está fría.");
        eprintln!("  Correr `bilinker check .` para calcularlos.");
    }
    Ok(())
}

// ─── graph ────────────────────────────────────────────────────────────────────

fn cmd_graph(root: &Path, cwd: &Path, selector: &str, format: &str, max_depth: Option<usize>, recursive: bool) -> anyhow::Result<()> {
    use std::collections::HashSet;

    let starts = find_graph_starts(root, cwd, selector, recursive)?;
    if starts.is_empty() {
        eprintln!("no bilinks found for '{selector}'");
        return Ok(());
    }

    let mut visited: HashSet<String> = HashSet::new();

    match format {
        "json" => graph_json(root, &starts)?,
        "flat" => {
            for (bilink_path, layer_root) in &starts {
                let bl = bilink_format::BiLink::load(bilink_path)?;
                let uuid = uuid_of(bilink_path);
                visited.insert(visit_key(&uuid, layer_root));
                graph_flat(root, &bl, &uuid, layer_root, &mut visited, 0, max_depth)?;
            }
        }
        _ => {
            println!("{selector}");
            if !starts.is_empty() { println!("│"); }
            for (i, (bilink_path, layer_root)) in starts.iter().enumerate() {
                let is_last = i == starts.len() - 1;
                let bl = bilink_format::BiLink::load(bilink_path)?;
                let uuid = uuid_of(bilink_path);
                visited.insert(visit_key(&uuid, layer_root));
                graph_tree(root, &bl, &uuid, layer_root, "", is_last, &mut visited, 0, max_depth)?;
                if !is_last { println!("│"); }
            }
        }
    }
    Ok(())
}

fn find_graph_starts(root: &Path, cwd: &Path, selector: &str, recursive: bool) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    // "." or "*" → all bilinks in current layer (or all layers if --recursive)
    if selector == "." || selector == "*" {
        let layers = if recursive {
            bilinker::index::layer_roots(root)
        } else {
            vec![cwd.to_path_buf()]
        };
        let mut starts = vec![];
        for layer in layers {
            let bilink_dir = layer.join(".bilink");
            if !bilink_dir.exists() { continue; }
            for entry in std::fs::read_dir(&bilink_dir)? {
                let path = entry?.path();
                if path.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
                if path.file_name().and_then(|n| n.to_str())
                    .map(|n| n.starts_with('.')).unwrap_or(false) { continue; }
                starts.push((path, layer.clone()));
            }
        }
        starts.sort_by(|a, b| a.0.cmp(&b.0));
        return Ok(starts);
    }

    // UUID or prefix → direct lookup in cwd's .bilink/
    let looks_like_uuid = selector.len() >= 8
        && !selector.contains('/')
        && !selector.contains('.')
        && selector.chars().all(|c| c.is_ascii_hexdigit() || c == '-');

    if looks_like_uuid {
        let bilink_path = bilinker::accept::find_bilink_path(&cwd.join(".bilink"), selector)?;
        return Ok(vec![(bilink_path, cwd.to_path_buf())]);
    }

    let file_str = selector.splitn(2, ':').next().unwrap_or(selector);
    let file_path = cwd.join(file_str);
    let results = bilinker::check::find_by_file(root, &file_path)?;

    Ok(results.into_iter().map(|(bilink_path, _n, _range)| {
        let layer_root = bilink_path.parent().and_then(|p| p.parent())
            .unwrap_or(cwd).to_path_buf();
        (bilink_path, layer_root)
    }).collect())
}

// ─── formato json: contrato de proveedor hacia lattice ───────────────────────

/// La raíz más externa del ecosistema que contiene a `start`.
///
/// La forma canónica de un nodo tiene que ser la misma sin importar desde qué
/// capa se invoque: una cadena que sube a la capa de specs no puede identificar
/// al mismo fragmento distinto según se corra desde impl o desde la raíz. Por eso
/// el label se calcula contra el ancestro más externo que sea repo o capa, y no
/// contra el directorio de invocación.
fn outermost_root(start: &Path) -> PathBuf {
    let mut best = start.to_path_buf();
    let mut cur  = start;
    while let Some(parent) = cur.parent() {
        if parent.join(".git").exists() || parent.join(".bilink").exists() {
            best = parent.to_path_buf();
        }
        cur = parent;
    }
    best
}

/// Un extremo estructural de una cadena, en forma canónica de lattice.
struct TipNode {
    canonical: String,
    state:     String,
    commit:    String,
}

/// Recorre la cadena de `bl` y devuelve sus extremos estructurales.
///
/// Los nodos intermedios son mecanismo interno de bilinker: si lattice los
/// viera, el grafo se llenaría de nodos `.bilink` que no son contenido del
/// proyecto. Por eso una cadena de N nodos emite **una** arista entre sus tips.
fn chain_tips(base: &Path, uuid: &str, layer_root: &Path) -> Vec<TipNode> {
    let mut tips    = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut queue   = vec![layer_root.to_path_buf()];

    while let Some(layer) = queue.pop() {
        if !visited.insert(visit_key(uuid, &layer)) { continue; }
        let Ok(node) = bilink_format::BiLink::load(&bilink_format::BiLink::path_in(&layer, uuid)) else { continue };
        let cache = bilinker::cache::Cache::load(&layer);

        for n in [0u8, 1u8] {
            let Some(id) = node.endpoint.get(n).link.capture_id() else { continue };
            let Ok(cap) = bilink_format::Capture::load_in(&layer, id) else { continue };
            // El rango sale de la cache. Con cache fría no hay nodo canónico que
            // emitir: lattice necesita `check` corrido antes de consultar.
            let Some(range) = cache.capture_ranges(id) else { continue };
            tips.push(TipNode {
                canonical: format!("{}::{}#{range}",
                    layer_label(base, &layer), cap.file),
                state:  cache.endpoint_state(uuid, n).map(|s| s.to_string()).unwrap_or_else(|| "—".into()),
                commit: cache.commit(uuid, n).unwrap_or("").to_string(),
            });
        }

        for (_, adj_layer) in layer_children(&node, uuid, &layer) {
            queue.push(adj_layer);
        }
    }
    tips
}

/// Emite las aristas de bilinker en el modelo de lattice.
fn graph_json(root: &Path, starts: &[(PathBuf, PathBuf)]) -> anyhow::Result<()> {
    use bilink_format::LinkEndpoint;
    let base     = outermost_root(root);
    let mut out  = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (path, layer_root) in starts {
        let Ok(bl) = bilink_format::BiLink::load(path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if !seen.insert(uuid.to_string()) { continue; }

        // El `kind` sale de la semántica declarada; sin `kind`, es un bilink.
        let kind = match (&bl.endpoint.zero.link, &bl.endpoint.one.link) {
            (LinkEndpoint::Issue(_), _) | (_, LinkEndpoint::Issue(_)) => "issue",
            _ => "bilink",
        };

        let tips = chain_tips(&base, uuid, layer_root);
        if tips.len() < 2 { continue; }
        let (a, b) = (&tips[0], &tips[1]);

        out.push(format!(
            r#"  {{"from":"{}","to":"{}","kind":"{}","guarantee":"accepted","provider":"bilinker","directed":false,"ref":"{}","state":["{}","{}"],"commit":["{}","{}"]}}"#,
            esc_json(&a.canonical), esc_json(&b.canonical),
            kind, uuid, a.state, b.state, a.commit, b.commit,
        ));
    }

    println!("[\n{}\n]", out.join(",\n"));
    Ok(())
}

fn layer_label(root: &Path, layer_root: &Path) -> String {
    let rel = layer_root.strip_prefix(root).unwrap_or(layer_root);
    if rel.as_os_str().is_empty() { ".".to_string() } else { rel.display().to_string() }
}

/// Escapa una cadena para embeberla en JSON.
fn esc_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
     .replace('\n', "\\n").replace('\r', "").replace('\t', "\\t")
}

/// El uuid de un bilink es el nombre de su archivo.
fn uuid_of(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string()
}

fn visit_key(uuid: &str, layer_root: &Path) -> String {
    format!("{}@{}", uuid, layer_root.display())
}

fn layer_children(bl: &bilink_format::BiLink, uuid: &str, layer_root: &Path) -> Vec<(PathBuf, PathBuf)> {
    use bilink_format::LinkEndpoint;
    let mut children = vec![];
    for n in [0u8, 1u8] {
        if let LinkEndpoint::Path(p) = &bl.endpoint.get(n).link {
            if let Ok(adj) = stratum::resolve(layer_root, layer_root, p.tokens()) {
                // Subir a la raíz verdadera de la capa vecina (.git o .bilink)
                let true_adj = bilinker::config::Config::load_from(&adj)
                    .map(|(r, _)| r)
                    .unwrap_or(adj);
                let adj_bilink = bilink_format::BiLink::path_in(&true_adj, uuid);
                if adj_bilink.exists() {
                    children.push((adj_bilink, true_adj));
                }
            }
        }
    }
    children
}

fn graph_tree(
    root: &Path,
    bl: &bilink_format::BiLink,
    uuid: &str,
    layer_root: &Path,
    prefix: &str,
    is_last: bool,
    visited: &mut std::collections::HashSet<String>,
    depth: usize,
    max_depth: Option<usize>,
) -> anyhow::Result<()> {
    let conn = if is_last { "└── " } else { "├── " };
    let ext  = if is_last { "    " } else { "│   " };
    let child_prefix = format!("{prefix}{ext}");

    let uuid_short = &uuid[..8.min(uuid.len())];
    let cache = bilinker::cache::Cache::load(layer_root);
    let st = |n: u8| cache.endpoint_state(uuid, n).map(|s| s.to_string()).unwrap_or_else(|| "—".into());
    let (s0, s1) = (st(0), st(1));
    let layer_label = if depth > 0 {
        let rel = layer_root.strip_prefix(root).unwrap_or(layer_root);
        format!("  ({})", rel.display())
    } else {
        String::new()
    };

    println!("{prefix}{conn}{uuid_short}  [{s0} ↔ {s1}]{layer_label}");
    println!("{child_prefix}│  endpoint.0  {}", bl.endpoint.zero.link);
    println!("{child_prefix}│  endpoint.1  {}", bl.endpoint.one.link);

    let children = if max_depth.map_or(true, |d| depth < d) {
        layer_children(bl, uuid, layer_root)
    } else {
        vec![]
    };

    if children.is_empty() {
        println!("{child_prefix}│");
    } else {
        println!("{child_prefix}│");
        for (i, (adj_bilink_path, adj_layer)) in children.iter().enumerate() {
            let key = visit_key(uuid, adj_layer);
            if visited.contains(&key) {
                let child_conn = if i == children.len() - 1 { "└── " } else { "├── " };
                println!("{child_prefix}{child_conn}{uuid_short}  [ya visitado]");
                continue;
            }
            visited.insert(key);
            let adj_bl = bilink_format::BiLink::load(adj_bilink_path)?;
            let child_is_last = i == children.len() - 1;
            graph_tree(root, &adj_bl, uuid, adj_layer, &child_prefix, child_is_last, visited, depth + 1, max_depth)?;
        }
    }
    Ok(())
}

fn graph_flat(
    root: &Path,
    bl: &bilink_format::BiLink,
    uuid: &str,
    layer_root: &Path,
    visited: &mut std::collections::HashSet<String>,
    depth: usize,
    max_depth: Option<usize>,
) -> anyhow::Result<()> {

    let uuid_short = &uuid[..8.min(uuid.len())];
    let cache = bilinker::cache::Cache::load(layer_root);
    let st = |n: u8| cache.endpoint_state(uuid, n).map(|s| s.to_string()).unwrap_or_else(|| "—".into());
    let (s0, s1) = (st(0), st(1));
    let layer_label = {
        let rel = layer_root.strip_prefix(root).unwrap_or(layer_root);
        if rel.as_os_str().is_empty() { ".".to_string() } else { rel.display().to_string() }
    };

    println!("{uuid_short}  {s0} ↔ {s1}  {}  →  {}  [{}]",
        bl.endpoint.zero.link, bl.endpoint.one.link, layer_label);

    if max_depth.map_or(true, |d| depth < d) {
        for (adj_bilink_path, adj_layer) in layer_children(bl, uuid, layer_root) {
            let key = visit_key(uuid, &adj_layer);
            if visited.contains(&key) { continue; }
            visited.insert(key);
            let adj_bl = bilink_format::BiLink::load(&adj_bilink_path)?;
            graph_flat(root, &adj_bl, uuid, &adj_layer, visited, depth + 1, max_depth)?;
        }
    }
    Ok(())
}

// ─── impact ───────────────────────────────────────────────────────────────────

fn line_col_to_byte(source: &str, line: usize, col: usize) -> usize {
    let mut cur_line = 1;
    let mut byte = 0;
    for (i, c) in source.char_indices() {
        if cur_line == line {
            return i + (col - 1).min(source.len() - i);
        }
        if c == '\n' { cur_line += 1; }
        byte = i;
    }
    byte
}

/// Stagea y commitea los archivos escritos. Devuelve el hash corto.
fn git_commit(root: &Path, paths: &[PathBuf], message: &str) -> anyhow::Result<String> {
    for path in paths {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let st = std::process::Command::new("git")
            .args(["add", &rel.display().to_string()])
            .current_dir(root)
            .status()?;
        if !st.success() {
            anyhow::bail!("git add falló para {}", path.display());
        }
    }
    let out = std::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(root)
        .output()?;
    if !out.status.success() {
        anyhow::bail!("git commit falló:\n{}", String::from_utf8_lossy(&out.stderr));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.lines().next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or("?")
        .to_string())
}
