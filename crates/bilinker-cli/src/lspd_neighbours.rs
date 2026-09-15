//! El vecindario resuelto contra [`lspd`](https://github.com/anibalanto/lspd).
//!
//! **Éste es el único archivo de bilinker que nombra al daemon.** La librería define
//! el puerto y no sabe quién lo implementa; el binario elige. No es para evitar un
//! ciclo —desde que el daemon salió de lattice no hay ninguno— sino para que
//! bilinker no quede atado a *ese* daemon: mañana puede ser SCIP, un índice propio,
//! o un language server hablado directo.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use bilinker::neighbours::{Location, Neighbours};

pub struct Lspd;

impl Neighbours for Lspd {
    /// Los tipos que la firma menciona, un salto.
    ///
    /// Devuelve `None` cuando no hay daemon y no se lo pudo levantar: **no pude
    /// mirar**, que es distinto de *no hay vecinos*. Se pregunta con un `ping`, que
    /// falla en el acto si el socket no está, y el arranque se intenta una sola vez por
    /// corrida — por eso `check` sin daemon paga a lo sumo un arranque fallido, y no
    /// uno por endpoint.
    ///
    /// **Y también cuando el daemon está pero el servidor de atrás sigue indexando.**
    /// Un `ping` contesta antes que el language server esté listo, así que "hay
    /// daemon" no alcanza: en esa ventana `definitions` devolvía `[]`, que llega como
    /// `Some(vec![])` —*"miré y ninguna de estas posiciones resuelve acá"*— y se
    /// escribe como vecindario adquirido. Es una cobertura afirmada que no existe.
    ///
    /// La distinción no se puede hacer de este lado: un vacío legítimo llega igual, y
    /// llega seguido —`Result<Checked>` pregunta por los dos y `Result` vive en otro
    /// crate—. Así que la da el daemon con [`NOT_READY`](lspd_client::NOT_READY) y acá
    /// sólo se traduce.
    ///
    /// **Las posiciones son identificadores de tipo**, no el primer byte de un campo
    /// de la firma. Es lo que hace que preguntar acá tenga sentido: sobre un `(` un
    /// language server que resuelve perfecto devuelve la función que lo contiene.
    /// **Y se le pregunta a la puerta de esta capa, no a "la" del sistema.**
    ///
    /// Medido el 2026-09-07: un `check` en la capa impl de worklist fallo con
    /// `file not found` sobre un archivo que existe, porque el daemon vivo
    /// estaba indexando otro proyecto. Un daemon ajeno **no contesta "no se":
    /// contesta que el archivo no existe**, y eso llegaba como un error del
    /// arbol.
    ///
    /// El arreglo de entonces fue deducir su workspace del `cwd` del pid —una
    /// costura, y de Linux—. **Con una puerta por workspace no hace falta**: el
    /// que contesta en mi puerta es el mio por construccion, y el chequeo se
    /// borro. Ver `concepts/transport.md` de lspd.
    fn of(&self, layer: &Path, file: &str, at: &[usize]) -> Result<Option<Vec<Location>>> {
        of_with(&Real, &RUN, Pace::REAL, &mut std::io::stderr(), layer, file, at)
    }
}

/// Lo que el adaptador le pide al daemon.
///
/// **Es un trait para que la política se pruebe sin socket**: cuándo se levanta,
/// cuándo se espera y cuándo se deja de esperar son decisiones de este archivo, y la
/// suite no puede depender de un `lspd` ni de un language server reales.
trait Daemon {
    fn responds(&self, layer: &Path) -> bool;
    fn spawn(&self, layer: &Path) -> Result<()>;
    fn definitions(&self, layer: &Path, abs: &str, line: usize, col: usize) -> Result<serde_json::Value>;
    fn status(&self, layer: &Path) -> Result<serde_json::Value>;
    fn warn_about_foreign_doors(&self, layer: &Path);
    /// Antes de cada espera. Es donde el real se hace cargo de Ctrl-C.
    fn before_waiting(&self) {}
}

/// El daemon de verdad, por `lspd-client`.
struct Real;

impl Daemon for Real {
    fn responds(&self, layer: &Path) -> bool { lspd_client::responds(layer) }
    fn spawn(&self, layer: &Path) -> Result<()> { lspd_client::spawn(layer).map(|_| ()) }
    fn definitions(&self, layer: &Path, abs: &str, line: usize, col: usize) -> Result<serde_json::Value> {
        lspd_client::rpc(layer, "definitions", serde_json::json!({
            "file": abs, "line": line, "col": col,
        }))
    }
    fn status(&self, layer: &Path) -> Result<serde_json::Value> {
        lspd_client::rpc(layer, "status", serde_json::json!({}))
    }
    fn warn_about_foreign_doors(&self, layer: &Path) { warn_about_foreign_doors(layer) }
    fn before_waiting(&self) { catch_interrupts(&RUN) }
}

/// Lo que esta corrida ya hizo con el daemon.
///
/// **Es estado de la corrida y no del endpoint**, por lo mismo que el aviso de
/// puertas ajenas: `check` pregunta por treinta endpoints, y treinta arranques o
/// treinta esperas no son lo que nadie pidió.
struct Run {
    /// Ya se intentó levantar el daemon.
    spawn_tried: AtomicBool,
    /// Y arrancó: el daemon es de esta corrida, y lo que indexe se espera.
    spawned:     AtomicBool,
    /// Alguien apretó Ctrl-C en una espera: el resto de la corrida sigue sin vecindario.
    gave_up:     AtomicBool,
    /// Hay una espera en curso, y Ctrl-C la corta en vez de matar la corrida.
    waiting:     AtomicBool,
    /// Llegó un Ctrl-C durante la espera.
    interrupted: AtomicBool,
}

impl Run {
    const fn new() -> Self {
        Run {
            spawn_tried: AtomicBool::new(false),
            spawned:     AtomicBool::new(false),
            gave_up:     AtomicBool::new(false),
            waiting:     AtomicBool::new(false),
            interrupted: AtomicBool::new(false),
        }
    }
}

static RUN: Run = Run::new();

/// Cada cuánto se consulta `status`, y cada cuánto se dice cómo va.
///
/// **Se dice menos seguido de lo que se pregunta.** Siete minutos de `rust-analyzer`
/// con una línea por segundo tapan la salida que la corrida vino a dar.
#[derive(Clone, Copy)]
struct Pace {
    every:        Duration,
    report_every: Duration,
}

impl Pace {
    const REAL: Pace = Pace { every: Duration::from_secs(1), report_every: Duration::from_secs(15) };
}

/// El vecindario, con el daemon que hay, el que se levanta, o ninguno.
fn of_with(
    daemon: &dyn Daemon,
    run:    &Run,
    pace:   Pace,
    out:    &mut dyn Write,
    layer:  &Path,
    file:   &str,
    at:     &[usize],
) -> Result<Option<Vec<Location>>> {
    if !ensure_daemon(daemon, run, layer) {
        return Ok(None);
    }

    let abs = layer.join(file);
    let source = std::fs::read_to_string(&abs)?;
    let abs = abs.to_string_lossy();

    let mut found: Vec<Location> = Vec::new();
    // **Las posiciones vienen dadas, no se deducen del rango.** Quién sabe dónde
    // hay un tipo es la gramática, y la gramática es de bilinker; acá sólo se
    // traduce a lo que el daemon entiende.
    for &byte in at {
        // El daemon habla en línea/columna 0-based, como LSP. La conversión es de
        // este lado: traducirla allá sería ponerle al daemon una convención que
        // no es suya.
        let (line, col) = line_col_of(&source, byte);
        // Cuántas veces seguidas el daemon dijo "todavía no" sin que `status` tuviera
        // nada que esperar.
        let mut idle_retries = 0;
        let val = loop {
            match daemon.definitions(layer, &abs, line, col) {
                Ok(v) => break v,
                // **Una posición sin resolver invalida el vecindario entero**, no
                // sólo la suya: el fold es sobre el conjunto, y un conjunto al que le
                // falta un miembro hashea distinto que el completo. Devolver lo que
                // se alcanzó a juntar sería exactamente el vacío que este camino
                // existe para no escribir.
                Err(e) if not_ready(&e) => {
                    // Un daemon que ya estaba vivo es de otra corrida: lo que indexa
                    // no se espera, y su "todavía no" es `None`.
                    if !run.spawned.load(Ordering::Relaxed) || run.gave_up.load(Ordering::Relaxed) {
                        return Ok(None);
                    }
                    match wait_ready(daemon, run, pace, out, layer) {
                        Waited::Ready { polled: true } => idle_retries = 0,
                        // `status` no tenía nada que esperar: pudo quedar listo entre
                        // las dos preguntas, así que se vuelve a preguntar una vez.
                        // Dos seguidas no es una carrera, y dar vueltas no lo arregla.
                        Waited::Ready { polled: false } => {
                            idle_retries += 1;
                            if idle_retries > 1 { return Ok(None) }
                        }
                        Waited::Interrupted | Waited::Gone => return Ok(None),
                    }
                }
                Err(e) => return Err(e),
            }
        };
        for d in val.as_array().into_iter().flatten() {
            let Some(loc) = location_of(layer, d) else { continue };
            found.push(loc);
        }
    }
    Ok(Some(found))
}

/// Si hay un daemon que contesta en la puerta de esta capa, levantándolo si hace
/// falta.
///
/// **Se intenta una vez por corrida.** Si no arranca, es `None` como siempre: un
/// daemon que no arranca es una falla de infraestructura, y no puede volverse un
/// error de `check` ni de `accept`.
///
/// **Y el aviso de puertas ajenas sale sólo cuando igual degrada.** Antes de
/// levantar sería ruido: si arranca, no hay desfasaje que explicar.
fn ensure_daemon(daemon: &dyn Daemon, run: &Run, layer: &Path) -> bool {
    if daemon.responds(layer) { return true }
    if !run.spawn_tried.swap(true, Ordering::Relaxed) && daemon.spawn(layer).is_ok() {
        run.spawned.store(true, Ordering::Relaxed);
        return true;
    }
    daemon.warn_about_foreign_doors(layer);
    false
}

/// Cómo terminó una espera.
#[derive(Debug, PartialEq, Eq)]
enum Waited {
    /// No queda nada indexando. `polled` dice si hubo que esperar algo.
    Ready { polled: bool },
    /// Ctrl-C.
    Interrupted,
    /// `status` dejó de contestar: el daemon se murió mientras se esperaba.
    Gone,
}

/// Espera a que ningún language server del daemon esté `INDEXING`.
///
/// **Se espera todo lo que indexa, y no "el servidor de este archivo".** `status`
/// nombra servidores, no lenguajes ni extensiones, y deducir cuál atiende un archivo
/// sería copiar de este lado una tabla que es del daemon. Con un daemon levantado
/// en esta corrida, lo que indexa lo arrancaron las preguntas de esta corrida.
///
/// **No tiene techo de tiempo.** Lo que la corta es Ctrl-C.
fn wait_ready(
    daemon: &dyn Daemon,
    run:    &Run,
    pace:   Pace,
    out:    &mut dyn Write,
    layer:  &Path,
) -> Waited {
    daemon.before_waiting();
    // `interrupted` no se limpia acá: el handler sólo lo prende durante una espera,
    // y una espera cortada deja `gave_up`, así que no hay una segunda.
    run.waiting.store(true, Ordering::Relaxed);
    let waited = wait_ready_inner(daemon, run, pace, out, layer);
    run.waiting.store(false, Ordering::Relaxed);
    if waited == Waited::Interrupted {
        run.gave_up.store(true, Ordering::Relaxed);
        let _ = writeln!(out, "  espera cortada: esta corrida sigue sin vecindario de tipos.");
    }
    waited
}

fn wait_ready_inner(
    daemon: &dyn Daemon,
    run:    &Run,
    pace:   Pace,
    out:    &mut dyn Write,
    layer:  &Path,
) -> Waited {
    let start = Instant::now();
    let mut last_report: Option<Instant> = None;
    let mut polled = false;
    loop {
        let Ok(status) = daemon.status(layer) else { return Waited::Gone };
        let pending = indexing(&status);
        if pending.is_empty() { return Waited::Ready { polled } }
        polled = true;

        if last_report.is_none_or(|t| t.elapsed() >= pace.report_every) {
            let _ = writeln!(out,
                "… esperando a {} (INDEXING, {}) para el vecindario de tipos. \
                 Ctrl-C sigue sin vecindario.",
                pending.join(", "), elapsed(start.elapsed()));
            last_report = Some(Instant::now());
        }

        if run.interrupted.load(Ordering::Relaxed) { return Waited::Interrupted }
        std::thread::sleep(pace.every);
        if run.interrupted.load(Ordering::Relaxed) { return Waited::Interrupted }
    }
}

/// Los servidores que `status` da `INDEXING`.
///
/// **Sólo esos se esperan.** `READY` ya contesta, y `RUNNING` es un servidor que no
/// informa readiness: no hay a qué esperar, y un vacío suyo vale lo que vale para
/// ese lenguaje.
fn indexing(status: &serde_json::Value) -> Vec<String> {
    status.as_array().into_iter().flatten()
        .filter(|s| s.get("state").and_then(|v| v.as_str()) == Some("INDEXING"))
        .map(|s| s.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string())
        .collect()
}

/// Cuánto va una espera, en lo que alguien lee de un vistazo.
fn elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 { format!("{s}s") } else { format!("{}m{:02}s", s / 60, s % 60) }
}

/// Se hace cargo de Ctrl-C, una vez por proceso.
///
/// **Durante una espera, Ctrl-C la corta y la corrida sigue.** Fuera de una espera
/// termina el proceso como terminaba antes, con 130. El handler se instala recién
/// en la primera espera: una corrida que no espera no cambia en nada.
fn catch_interrupts(run: &'static Run) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = ctrlc::set_handler(move || {
            if run.waiting.load(Ordering::Relaxed) {
                run.interrupted.store(true, Ordering::Relaxed);
            } else {
                std::process::exit(130);
            }
        });
    });
}

/// Ya se avisó del desfasaje en esta corrida.
///
/// **El aviso es por corrida y no por endpoint.** `check` es masivo: en la capa de
/// worklist son treinta endpoints con vecindario, y treinta líneas iguales dicen lo
/// mismo que una y tapan el resto de la salida.
static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Cuando mi puerta no contesta, avisa si hay **otras** que sí.
///
/// **"No hay daemon" y "hay uno y no lo veo" se ven igual, y no son lo mismo.** La
/// ruta se deriva de una regla que vive en el codigo de las dos puntas, asi que
/// versionarla mal las parte en dos: el daemon abre una puerta y el cliente calcula
/// otra. Y no encontrarla es exactamente lo que pasa cuando no hay ninguno.
///
/// Medido el 2026-09-08: el nombre de la puerta cambio en lspd, el commit quedo sin
/// publicar, y `bilinker` —que toma `lspd-client` del remoto de git— siguio buscando
/// el nombre viejo. Treinta endpoints degradaron a *no verificado* sin una linea que
/// dijera por que. Ver `concepts/transport.md` de lspd.
///
/// **Se pregunta con el mismo `ping`, no mirando procesos.** Un socket vivo contesta
/// y uno stale falla en el acto; deducirlo del pid seria la costura de `/proc` que
/// esto justamente vino a borrar.
fn warn_about_foreign_doors(layer: &Path) {
    use std::sync::atomic::Ordering;
    if WARNED.swap(true, Ordering::Relaxed) { return }

    let Some(mine) = lspd_client::endpoint(layer).path().map(|p| p.to_path_buf())
    else { return };  // En Windows la puerta es un pipe y no hay directorio que mirar.

    let Ok(entries) = std::fs::read_dir(lspd_client::dir()) else { return };
    let foreign: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p != &mine && p.extension().and_then(|s| s.to_str()) == Some("sock"))
        .filter(|p| {
            let ep = lspd_client::Endpoint::Socket(p.clone());
            lspd_client::rpc_at(&ep, "ping", serde_json::json!({})).is_ok()
        })
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();

    if foreign.is_empty() { return }
    eprintln!(
        "! el daemon de esta capa no contesta y hay {} puerta(s) viva(s) que este \
         cliente no calcula: {}\n  \
         el nivel 1 va a degradar a no verificado. Si `lspd` y `lspd-client` no son \
         de la misma version, la regla del nombre de la puerta los partio en dos.",
        foreign.len(),
        foreign.join(", "),
    );
}

/// Si el daemon contestó *"todavía no puedo"*.
///
/// **Por código y no por mensaje.** El texto del error es prosa que alguien va a
/// mejorar; el código es el contrato.
fn not_ready(e: &anyhow::Error) -> bool {
    e.downcast_ref::<lspd_client::RpcError>().is_some_and(|r| r.is_not_ready())
}

/// Una definición del daemon, traducida a la forma que bilinker foldea.
///
/// Se descarta lo que cae fuera de la capa: el vecindario de un contrato son los
/// tipos del proyecto, y un `String` de la stdlib no es algo que nadie vaya a
/// aceptar ni a mirar cuando cambie.
fn location_of(layer: &Path, d: &serde_json::Value) -> Option<Location> {
    let file   = d.get("file")?.as_str()?;
    let symbol = d.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let rel = Path::new(file).strip_prefix(layer).ok()?.to_string_lossy().to_string();

    let source = std::fs::read_to_string(file).ok()?;
    let start = byte_of(&source, d.get("line")?.as_u64()? as usize,
                                 d.get("col")?.as_u64()? as usize)?;
    let end = d.get("end_line").and_then(|v| v.as_u64())
        .zip(d.get("end_col").and_then(|v| v.as_u64()))
        .and_then(|(l, c)| byte_of(&source, l as usize, c as usize))
        .unwrap_or(source.len());

    Some(Location { file: rel, symbol, start, end })
}

/// Línea y columna 0-based de un offset, contando bytes.
fn line_col_of(source: &str, byte: usize) -> (usize, usize) {
    let end = byte.min(source.len());
    let head = &source.as_bytes()[..end];
    let line = head.iter().filter(|&&b| b == b'\n').count();
    let col  = end - head.iter().rposition(|&b| b == b'\n').map(|i| i + 1).unwrap_or(0);
    (line, col)
}

fn byte_of(source: &str, line: usize, col: usize) -> Option<usize> {
    let mut at = 0usize;
    for (i, l) in source.split_inclusive('\n').enumerate() {
        if i == line { return Some((at + col).min(source.len())); }
        at += l.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "uno\ndos\ntres\n";

    #[test]
    fn line_and_col_round_trip() {
        for byte in [0usize, 3, 4, 7, 8, 11] {
            let (l, c) = line_col_of(SRC, byte);
            assert_eq!(byte_of(SRC, l, c), Some(byte), "byte {byte} → {l}:{c}");
        }
    }

    /// Cuenta bytes y no caracteres: una `ó` en una spec en castellano alcanza para
    /// que las dos cosas dejen de coincidir.
    #[test]
    fn it_counts_bytes() {
        let src = "canción\nsiguiente\n";
        let byte = src.find("siguiente").unwrap();
        let (l, c) = line_col_of(src, byte);
        assert_eq!((l, c), (1, 0));
        assert_eq!(byte_of(src, l, c), Some(byte));
    }

    // ─── el daemon que falta, y la espera ────────────────────────────────────

    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    /// Un daemon de mentira: la suite no le habla a un `lspd` real.
    ///
    /// `answers` son las respuestas a `definitions`, en orden —un `Err` lleva el
    /// código—, y `statuses` las de `status`, donde la última se repite.
    struct Fake {
        alive:        Cell<bool>,
        spawn_ok:     bool,
        spawns:       Cell<u32>,
        foreign:      Cell<u32>,
        answers:      RefCell<VecDeque<std::result::Result<serde_json::Value, i32>>>,
        statuses:     RefCell<VecDeque<std::result::Result<serde_json::Value, ()>>>,
        status_calls: Cell<u32>,
    }

    impl Fake {
        fn new(alive: bool, spawn_ok: bool) -> Self {
            Fake {
                alive: Cell::new(alive), spawn_ok,
                spawns: Cell::new(0), foreign: Cell::new(0),
                answers: RefCell::new(VecDeque::new()),
                statuses: RefCell::new(VecDeque::new()),
                status_calls: Cell::new(0),
            }
        }
        fn answering(self, a: Vec<std::result::Result<serde_json::Value, i32>>) -> Self {
            *self.answers.borrow_mut() = a.into(); self
        }
        fn with_status(self, s: Vec<std::result::Result<serde_json::Value, ()>>) -> Self {
            *self.statuses.borrow_mut() = s.into(); self
        }
    }

    impl Daemon for Fake {
        fn responds(&self, _: &Path) -> bool { self.alive.get() }
        fn spawn(&self, _: &Path) -> Result<()> {
            self.spawns.set(self.spawns.get() + 1);
            if self.spawn_ok { self.alive.set(true); Ok(()) }
            else { anyhow::bail!("no se pudo arrancar el daemon (lspd): not found") }
        }
        fn definitions(&self, _: &Path, _: &str, _: usize, _: usize) -> Result<serde_json::Value> {
            match self.answers.borrow_mut().pop_front().expect("una pregunta de más") {
                Ok(v) => Ok(v),
                Err(code) => Err(lspd_client::RpcError { code, message: "no".into() }.into()),
            }
        }
        fn status(&self, _: &Path) -> Result<serde_json::Value> {
            self.status_calls.set(self.status_calls.get() + 1);
            let mut s = self.statuses.borrow_mut();
            let next = if s.len() > 1 { s.pop_front().unwrap() } else { s.front().cloned().expect("sin status") };
            next.map_err(|_| anyhow::anyhow!("no hay daemon"))
        }
        fn warn_about_foreign_doors(&self, _: &Path) { self.foreign.set(self.foreign.get() + 1) }
    }

    const NOW: Pace = Pace { every: Duration::ZERO, report_every: Duration::ZERO };

    fn a_layer() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rs"), "pub fn f(x: Dto) {}\n").unwrap();
        d
    }

    fn server(name: &str, state: &str) -> serde_json::Value {
        serde_json::json!([{ "name": name, "state": state, "queries": 0 }])
    }

    fn ask(fake: &Fake, run: &Run, out: &mut Vec<u8>, layer: &Path) -> Result<Option<Vec<Location>>> {
        of_with(fake, run, NOW, out, layer, "a.rs", &[10])
    }

    /// **Un daemon que no arranca es `None`, no un error**: el vecindario queda no
    /// verificado y la corrida sigue. Y se intenta una vez por corrida, no una por
    /// endpoint.
    #[test]
    fn a_daemon_that_does_not_start_is_none_and_is_tried_once() {
        let d = a_layer();
        let (fake, run, mut out) = (Fake::new(false, false), Run::new(), Vec::new());
        assert!(ask(&fake, &run, &mut out, d.path()).unwrap().is_none());
        assert!(ask(&fake, &run, &mut out, d.path()).unwrap().is_none());
        assert_eq!(fake.spawns.get(), 1, "treinta endpoints no son treinta arranques");
        assert_eq!(fake.foreign.get(), 2, "el aviso de puertas ajenas sigue donde degrada");
    }

    #[test]
    fn a_daemon_that_does_not_answer_is_raised_and_asked() {
        let d = a_layer();
        let fake = Fake::new(false, true).answering(vec![Ok(serde_json::json!([]))]);
        let (run, mut out) = (Run::new(), Vec::new());
        assert_eq!(ask(&fake, &run, &mut out, d.path()).unwrap(), Some(vec![]));
        assert_eq!(fake.spawns.get(), 1);
        assert_eq!(fake.foreign.get(), 0, "no degradó: no hay nada que avisar");
    }

    #[test]
    fn a_live_daemon_is_not_raised() {
        let d = a_layer();
        let fake = Fake::new(true, true).answering(vec![Ok(serde_json::json!([]))]);
        let (run, mut out) = (Run::new(), Vec::new());
        assert_eq!(ask(&fake, &run, &mut out, d.path()).unwrap(), Some(vec![]));
        assert_eq!(fake.spawns.get(), 0);
    }

    /// Un daemon que ya estaba vivo es de otra corrida: su `-32001` no se espera.
    #[test]
    fn not_ready_from_a_daemon_that_was_already_alive_is_none_without_waiting() {
        let d = a_layer();
        let fake = Fake::new(true, true)
            .answering(vec![Err(lspd_client::NOT_READY)])
            .with_status(vec![Ok(server("rust-analyzer", "INDEXING"))]);
        let (run, mut out) = (Run::new(), Vec::new());
        assert!(ask(&fake, &run, &mut out, d.path()).unwrap().is_none());
        assert_eq!(fake.status_calls.get(), 0);
        assert!(out.is_empty());
    }

    /// El que levantó esta corrida sí se espera: `status` hasta `READY`, y se vuelve
    /// a preguntar. Y mientras, se dice qué se espera.
    #[test]
    fn not_ready_after_raising_waits_for_ready_and_asks_again() {
        let d = a_layer();
        let fake = Fake::new(false, true)
            .answering(vec![Err(lspd_client::NOT_READY), Ok(serde_json::json!([]))])
            .with_status(vec![
                Ok(server("rust-analyzer", "INDEXING")),
                Ok(server("rust-analyzer", "INDEXING")),
                Ok(server("rust-analyzer", "READY")),
            ]);
        let (run, mut out) = (Run::new(), Vec::new());
        assert_eq!(ask(&fake, &run, &mut out, d.path()).unwrap(), Some(vec![]));
        assert_eq!(fake.status_calls.get(), 3);
        let said = String::from_utf8(out).unwrap();
        assert!(said.contains("rust-analyzer"), "qué servidor:\n{said}");
        assert!(said.contains("INDEXING"), "en qué estado:\n{said}");
        assert!(said.contains("0s"), "cuánto va:\n{said}");
        assert!(said.contains("Ctrl-C"), "cómo se sigue sin vecindario:\n{said}");
    }

    /// Ctrl-C corta la espera, da `None`, y la corrida sigue sin volver a esperar.
    #[test]
    fn ctrl_c_ends_the_wait_with_none_and_the_run_goes_on() {
        let d = a_layer();
        let fake = Fake::new(false, true)
            .answering(vec![Err(lspd_client::NOT_READY), Err(lspd_client::NOT_READY)])
            .with_status(vec![Ok(server("rust-analyzer", "INDEXING"))]);
        let (run, mut out) = (Run::new(), Vec::new());
        run.interrupted.store(true, Ordering::Relaxed);
        assert!(ask(&fake, &run, &mut out, d.path()).unwrap().is_none());
        let polls = fake.status_calls.get();
        assert!(ask(&fake, &run, &mut out, d.path()).unwrap().is_none());
        assert_eq!(fake.status_calls.get(), polls, "después de Ctrl-C no se vuelve a esperar");
    }

    #[test]
    fn a_daemon_that_dies_while_waiting_is_none() {
        let d = a_layer();
        let fake = Fake::new(false, true)
            .answering(vec![Err(lspd_client::NOT_READY)])
            .with_status(vec![Err(())]);
        let (run, mut out) = (Run::new(), Vec::new());
        assert!(ask(&fake, &run, &mut out, d.path()).unwrap().is_none());
    }

    /// Si `status` no tiene nada que esperar y la pregunta insiste en `-32001`, no se
    /// queda dando vueltas: una vez se vuelve a preguntar, dos es `None`.
    #[test]
    fn not_ready_with_nothing_to_wait_for_does_not_spin() {
        let d = a_layer();
        let fake = Fake::new(false, true)
            .answering(vec![Err(lspd_client::NOT_READY), Err(lspd_client::NOT_READY)])
            .with_status(vec![Ok(server("rust-analyzer", "READY"))]);
        let (run, mut out) = (Run::new(), Vec::new());
        assert!(ask(&fake, &run, &mut out, d.path()).unwrap().is_none());
    }

    /// Sólo `INDEXING` se espera: `RUNNING` no informa readiness, y no hay a qué.
    #[test]
    fn only_an_indexing_server_is_waited() {
        let status = serde_json::json!([
            { "name": "rust-analyzer",              "state": "INDEXING", "queries": 0 },
            { "name": "typescript-language-server", "state": "RUNNING",  "queries": 3 },
            { "name": "jdtls",                      "state": "READY",    "queries": 1 },
        ]);
        assert_eq!(indexing(&status), vec!["rust-analyzer".to_string()]);
        assert!(indexing(&server("typescript-language-server", "RUNNING")).is_empty());
        assert!(indexing(&serde_json::json!([])).is_empty());
    }

    #[test]
    fn a_waiting_time_reads_in_minutes_and_seconds() {
        assert_eq!(elapsed(Duration::from_secs(0)), "0s");
        assert_eq!(elapsed(Duration::from_secs(42)), "42s");
        assert_eq!(elapsed(Duration::from_secs(7 * 60 + 2)), "7m02s");
    }
}

