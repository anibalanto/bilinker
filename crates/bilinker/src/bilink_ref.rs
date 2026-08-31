//! `refs/bilink/<branch>` — la ref donde viven los bilinks.
//!
//! Ninguna rama del proyecto contiene `.bilink/`. Cada commit de la ref lleva **el
//! árbol del proyecto más `.bilink/`**, así que es un snapshot consistente por
//! construcción y el consumidor remoto trae una sola ref.
//!
//! Acá está lo que todo comando que escribe sobre la ref tiene que cumplir:
//!
//! - **La invariante de fidelidad.** El árbol de código de todo commit de la ref es
//!   idéntico al del commit del proyecto absorbido más recientemente, y el commit
//!   contra el cual se calculó el acto tiene que estar absorbido antes de commitear.
//!   Absorber no es un comportamiento por comando: es **precondición de todo commit
//!   sobre la ref**, y cuando no se cumple se cumple absorbiendo en **un commit
//!   propio, inmediatamente antes**.
//! - **Un commit hace una cosa.** Trae código, o decide, o sincroniza decisiones;
//!   nunca dos de las tres. De ahí que acá haya dos puertas y no una:
//!   [`Repo::absorb`] y [`Repo::decide`]. [`Repo::classify`] es la vuelta de lectura
//!   —qué hizo un commit ya escrito— y la que un `pre-receive` puede correr con git
//!   a secas, sin abrir un bilink.
//! - **El índice propio.** `GIT_INDEX_FILE` sobre el mismo árbol de trabajo, así el
//!   mismo `.bilink/` queda ignorado por el índice del proyecto y trackeado por el
//!   de bilinker.
//! - **`.bilink/head`.** De qué rama y de qué commit salió el `.bilink/` del árbol.
//!   Lo escriben tanto la materialización como todo commit sobre la ref.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config;
use crate::refmsg::{RefCommand, RefMessage};

/// Lo que queda fuera del índice de bilinker, no sólo del índice del proyecto.
///
/// `cache/` e `index/` son derivados; `head` es estado del árbol de trabajo, no
/// contenido. Ninguno se commitea.
///
/// **La lista es la regla, y no hay ningún `.gitignore` detrás.** El árbol del
/// commit se construye enumerando, así que lo que no está acá entra — y lo que no
/// tiene que entrar se saca de acá, no agregando una línea a un archivo. Un
/// `.gitignore` para esto sería además una escritura versionada para resolver algo
/// que es del índice, y la exclusión del lado del proyecto ya la puso `init` en
/// `.git/info/exclude`, una sola vez y por clon.
const NOT_COMMITTED: [&str; 3] = ["cache", "index", "head"];

/// El repo, su rama, y la ref que le corresponde.
pub struct Repo {
    pub root: PathBuf,
    git_dir:  PathBuf,
}

impl Repo {
    /// Abre el repo que contiene `dir`. No exige que `init` haya corrido.
    pub fn open(dir: &Path) -> Result<Self> {
        let root = config::repo_root(dir)?;
        let git_dir = config::git_dir(&root)?;
        Ok(Self { root, git_dir })
    }

    /// La rama del proyecto checkouteada, o `None` con `HEAD` desacoplado.
    ///
    /// A mitad de un rebase con conflictos no hay rama contra la cual comparar, y
    /// adivinar una sería peor que no hacer nada.
    pub fn branch(&self) -> Option<String> {
        let out = self.git(&["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()?;
        let b = out.trim();
        (!b.is_empty()).then(|| b.to_string())
    }

    /// La rama actual, o el error que corresponde a `HEAD` desacoplado.
    pub fn require_branch(&self) -> Result<String> {
        self.branch().context(
            "HEAD está desacoplado: no hay rama actual contra la cual comparar.\n  \
             Volver a una rama antes de commitear sobre la ref.",
        )
    }

    pub fn ref_name(branch: &str) -> String {
        format!("refs/bilink/{branch}")
    }

    /// El tip de `refs/bilink/<branch>`, o `None` si la rama no tiene ref.
    ///
    /// Crear la ref de una rama es decidir de quién hereda los bilinks, y eso es
    /// trabajo de `track`, no de los demás comandos.
    pub fn ref_tip(&self, branch: &str) -> Option<String> {
        self.rev_parse(&Self::ref_name(branch))
    }

    pub fn require_ref_tip(&self, branch: &str) -> Result<String> {
        self.ref_tip(branch).with_context(|| {
            format!(
                "{} no existe.\n  Correr `bilinker track {branch}` para crearla.",
                Self::ref_name(branch)
            )
        })
    }

    /// El tip de la rama del proyecto.
    pub fn branch_tip(&self, branch: &str) -> Result<String> {
        self.rev_parse(branch)
            .with_context(|| format!("la rama {branch} no tiene commits"))
    }

    /// El commit del proyecto que un commit de la ref tiene absorbido.
    ///
    /// Es el segundo padre del merge más cercano siguiendo primeros padres, o el
    /// propio commit si él *es* ese merge. Un commit de un solo padre lo hereda del
    /// merge más cercano hacia atrás, que es la misma lectura y el mismo recorrido.
    ///
    /// El corte es el único commit sin ninguno por debajo: nace del `X` del proyecto
    /// como padre único, y ahí la fidelidad se lee contra `X` mismo.
    ///
    /// **El walk se frena al salir de la ref**, y el freno es la disyunción: los
    /// commits de la ref llevan `.bilink/` en su árbol y los del proyecto no. Sin
    /// ese freno el corte daría la respuesta equivocada — seguiría hacia atrás por
    /// la historia del proyecto y devolvería el segundo padre de un merge ajeno.
    pub fn absorbed(&self, ref_commit: &str) -> Result<Option<String>> {
        let mut current = ref_commit.to_string();
        loop {
            if !self.tree_has_bilink(&current)? {
                return Ok(Some(current));
            }
            let line = self.git(&["rev-list", "--parents", "-n", "1", &current])?;
            let mut parts = line.split_whitespace();
            parts.next();
            let parents: Vec<&str> = parts.collect();
            match parents.as_slice() {
                [_, second, ..] => return Ok(Some((*second).to_string())),
                [first] => current = (*first).to_string(),
                [] => return Ok(None),
            }
        }
    }

    /// Los commits **propios** de la ref, del más nuevo al corte.
    ///
    /// `git log --first-parent` sobre la ref no alcanza: al llegar al corte sigue
    /// hacia atrás por la historia del proyecto, porque el corte tiene un commit del
    /// proyecto como padre único. El freno es el mismo que el de [`Self::absorbed`]
    /// —los commits de la ref llevan `.bilink/` en su árbol y los del proyecto no—
    /// y ésta es la primitiva donde se dice una sola vez.
    ///
    /// Es también lo que `git log --first-parent` muestra como registro de
    /// decisiones, acotado a lo que es de la ref.
    pub fn ref_chain(&self, branch: &str) -> Result<Vec<String>> {
        let tip = self.require_ref_tip(branch)?;
        let chain = self.git(&["rev-list", "--first-parent", &tip])?;
        let mut out = Vec::new();
        for commit in chain.lines() {
            if !self.tree_has_bilink(commit)? {
                break;
            }
            out.push(commit.to_string());
        }
        Ok(out)
    }

    /// Si el árbol de un commit contiene algún `.bilink/`. Es el test de la
    /// disyunción, y también el freno del walk de [`Self::absorbed`].
    fn tree_has_bilink(&self, commit: &str) -> Result<bool> {
        let tree = self.git(&["ls-tree", "-r", "--name-only", commit])?;
        Ok(tree.lines().any(is_bilink_path))
    }

    // ─── Las dos verificaciones previas ──────────────────────────────────────

    /// **Disyunción.** El árbol del commit del proyecto no contiene `.bilink/`.
    ///
    /// Va sobre el **árbol** y no sobre el diff a propósito: el commit que *borra*
    /// `.bilink/` tiene un diff que lo toca y un árbol que no, y es exactamente el
    /// commit que hay que poder absorber — el `X` del corte es eso.
    ///
    /// No es exigible como invariante —nadie puede impedir que alguien mergee— pero
    /// sí detectable antes de que contamine nada.
    pub fn verify_disjoint(&self, project_commit: &str) -> Result<()> {
        let tree = self.git(&["ls-tree", "-r", "--name-only", project_commit])?;
        let offenders: Vec<&str> = tree.lines().filter(|p| is_bilink_path(p)).take(3).collect();
        if offenders.is_empty() {
            return Ok(());
        }
        bail!(
            "el árbol de {} contiene .bilink/ ({}…)\n\n  \
             Alguien mergeó la ref al proyecto, o commiteó bilinks a mano.\n  \
             Absorberlo haría que el árbol de la ref contenga dos .bilink/ que git\n  \
             fusionaría sin que nadie mire.\n\n  No se escribió nada.",
            short(project_commit),
            offenders.join(", ")
        );
    }

    /// **Fidelidad.** El árbol de código del commit nuevo es idéntico al del commit
    /// del proyecto absorbido. Comparación de tree oids: exacta, sin tree-sitter y
    /// sin abrir un blob.
    ///
    /// Vale por construcción —el árbol se arma con `read-tree` del absorbido más
    /// `update-index` de `.bilink/`— y se verifica igual, porque es la propiedad de
    /// la que depende todo consumidor remoto.
    pub fn verify_faithful(&self, new_tree: &str, absorbed: &str) -> Result<()> {
        let diff = self.git(&["diff-tree", "-r", "--name-only", absorbed, new_tree])?;
        let strays: Vec<&str> = diff
            .lines()
            .filter(|l| !l.trim().is_empty() && !is_bilink_path(l))
            .take(3)
            .collect();
        if strays.is_empty() {
            return Ok(());
        }
        bail!(
            "el árbol de código no coincide con {} ({}…)\n  \
             Es la invariante de fidelidad; no se escribió nada.",
            short(absorbed),
            strays.join(", ")
        );
    }

    // ─── El commit sobre la ref ──────────────────────────────────────────────

    /// **Absorción.** Trae el tip de la rama del proyecto a la ref y nada más.
    ///
    /// Dos padres —el de la ref y el del proyecto—, el árbol de código del segundo, y
    /// el `.bilink/` **del primero**: por eso su diff contra el primer padre es vacío
    /// por construcción, que es lo que la vuelve reconocible con git a secas.
    ///
    /// Que el `.bilink/` salga de la ref y no del árbol de trabajo es lo que permite
    /// absorber con escrituras pendientes en disco: la absorción no las arrastra, y
    /// la decisión que viene atrás las lleva enteras.
    ///
    /// Devuelve `None` cuando el tip ya estaba absorbido — no hay nada que traer, y
    /// un merge con el mismo segundo padre no diría nada nuevo.
    pub fn absorb(&self, branch: &str) -> Result<Option<Commit>> {
        let ref_tip = self.require_ref_tip(branch)?;
        let project_tip = self.branch_tip(branch)?;
        let absorbed = self.absorbed(&ref_tip)?.unwrap_or_else(|| ref_tip.clone());

        if absorbed == project_tip {
            return Ok(None);
        }

        self.verify_disjoint(&project_tip)?;

        let tree = self.build_tree_inheriting(&project_tip, &ref_tip)?;
        self.verify_faithful(&tree, &project_tip)?;

        let message = RefMessage::new(RefCommand::Absorb { project: project_tip.clone() })
            .with_prose(format!("{branch} al día"));
        let sha = self.write_ref_commit(
            branch,
            &tree,
            &[ref_tip, project_tip.clone()],
            &message.render(),
        )?;
        self.write_head(branch, &sha)?;

        Ok(Some(Commit { sha, absorbed: Some(project_tip), wrote: true }))
    }

    /// **Decisión.** Escribe el `.bilink/` del árbol de trabajo sobre el árbol de
    /// código que la ref ya tiene.
    ///
    /// ```text
    /// read-tree     <commit del proyecto absorbido>
    /// update-index  únicamente .bilink/
    /// ```
    ///
    /// **Un solo padre, siempre.** Nada del árbol de trabajo fuera de `.bilink/` entra
    /// jamás, y traer código no es de acá: si el proyecto se movió, [`Self::absorb`]
    /// lo trajo en un commit anterior. Que el tip esté absorbido es precondición y se
    /// verifica; no se cumple absorbiendo de contrabando.
    pub fn decide(&self, branch: &str, message: &RefMessage) -> Result<Commit> {
        let ref_tip = self.require_ref_tip(branch)?;
        let project_tip = self.branch_tip(branch)?;
        let absorbed = self.absorbed(&ref_tip)?.unwrap_or_else(|| ref_tip.clone());

        if absorbed != project_tip {
            bail!(
                "{} no está absorbido; la ref está en {}.\n                   Una decisión no absorbe: sobre la ref un commit hace una cosa.\n                   Absorber primero — `bilinker sync`. No se escribió nada.",
                short(&project_tip),
                short(&absorbed)
            );
        }

        let tree = self.build_tree(&absorbed)?;
        self.verify_faithful(&tree, &absorbed)?;

        let previous_tree = self.git(&["rev-parse", &format!("{ref_tip}^{{tree}}")])?;
        if tree.trim() == previous_tree.trim() {
            return Ok(Commit { sha: ref_tip, absorbed: None, wrote: false });
        }

        let sha = self.write_ref_commit(branch, &tree, &[ref_tip], &message.render())?;
        self.write_head(branch, &sha)?;

        Ok(Commit { sha, absorbed: None, wrote: true })
    }

    // ─── Un commit hace una cosa ─────────────────────────────────────────────

    /// Qué hizo un commit de la ref, leído **con git a secas**.
    ///
    /// Los tres tipos se separan por la cantidad de padres, de dónde vienen, y cuál de
    /// los dos árboles se movió contra el primer padre. Ni tree-sitter ni abrir un
    /// bilink: es exactamente lo que un `pre-receive` puede correr sin instalar nada.
    ///
    /// **Falla cuando el commit hace dos cosas** — absorbe y decide a la vez, o
    /// sincroniza y mueve código. Es la invariante 4 de `concepts/ref.md`, y éste es
    /// el único lugar donde se decide.
    pub fn classify(&self, commit: &str) -> Result<Act> {
        let line = self.git(&["rev-list", "--parents", "-n", "1", commit])?;
        let parents: Vec<String> =
            line.split_whitespace().skip(1).map(str::to_string).collect();

        let first = parents.first();
        let (bilink_moved, code_moved) = match first {
            Some(p) => self.what_moved(p, commit)?,
            // Un commit raíz: todo su árbol es diff. No hay ref antes del corte.
            None => (true, true),
        };

        match parents.as_slice() {
            // El corte: nace de un commit del proyecto como padre único. Es el único
            // commit de la ref cuyo primer padre no es de la ref.
            [x] if !self.tree_has_bilink(x)? => Ok(Act::Cut { project: x.clone() }),

            [_] => {
                if code_moved {
                    bail!(
                        "{} decide y mueve código a la vez: el árbol de código de una \
                         decisión es el de la absorción que tiene arriba",
                        short(commit)
                    );
                }
                Ok(Act::Decision)
            }

            [_, second, ..] if !self.tree_has_bilink(second)? => {
                if bilink_moved {
                    bail!(
                        "{} absorbe y decide a la vez: sobre la ref un commit hace una \
                         cosa, y el diff de .bilink/ de una absorción es vacío",
                        short(commit)
                    );
                }
                Ok(Act::Absorption { project: second.clone() })
            }

            [_, _, ..] => {
                if code_moved {
                    bail!(
                        "{} sincroniza y mueve código a la vez: los dos lados de una \
                         sincronización describen el mismo código",
                        short(commit)
                    );
                }
                Ok(Act::Synchronization)
            }

            [] => bail!("{} no tiene padres: no es un commit de la ref", short(commit)),
        }
    }

    // ─── lo que `verify-ref` necesita leer, y nada más ───────────────────────

    /// Si el repo está configurado para firmar sus commits.
    fn signs(&self) -> bool {
        self.git(&["config", "--bool", "commit.gpgsign"])
            .map(|v| v.trim() == "true")
            .unwrap_or(false)
    }

    /// Si `a` es antepasado de `b`. Es la definición de fast-forward.
    pub fn is_ancestor(&self, a: &str, b: &str) -> Result<bool> {
        let out = Command::new("git")
            .args(["-C", &self.root.to_string_lossy(), "merge-base", "--is-ancestor", a, b])
            .output()?;
        Ok(out.status.success())
    }

    /// Los padres de un commit, en orden.
    pub fn parents(&self, commit: &str) -> Result<Vec<String>> {
        let line = self.git(&["rev-list", "--parents", "-n", "1", commit])?;
        Ok(line.split_whitespace().skip(1).map(str::to_string).collect())
    }

    /// Público para `verify-ref`: si el árbol de un commit lleva algún `.bilink/`.
    pub fn tree_has_any_bilink(&self, commit: &str) -> Result<bool> {
        self.tree_has_bilink(commit)
    }

    /// Los archivos bajo `.bilink/` que el commit tocó, con su estado de git.
    ///
    /// Sin padre —el commit raíz— todo su árbol cuenta como agregado.
    pub fn changed_bilink_files(
        &self,
        base: Option<&str>,
        commit: &str,
    ) -> Result<Vec<(char, String)>> {
        let out = match base {
            Some(b) => self.git(&["diff-tree", "-r", "--no-renames", "--name-status", b, commit])?,
            None => self.git(&["ls-tree", "-r", "--name-only", commit])?,
        };
        let mut files = Vec::new();
        for line in out.lines().filter(|l| !l.trim().is_empty()) {
            let (status, path) = match base {
                Some(_) => match line.split_once('\t') {
                    Some((st, p)) => (st.chars().next().unwrap_or('M'), p),
                    None => continue,
                },
                None => ('A', line),
            };
            if is_bilink_path(path) {
                files.push((status, path.to_string()));
            }
        }
        Ok(files)
    }

    /// Un bilink leído del árbol de un commit, sin tocar el disco.
    pub fn bilink_at(&self, commit: &str, path: &str) -> Result<crate::BiLinkFile> {
        let text = self.git(&["show", &format!("{commit}:{path}")])?;
        Ok(serde_yaml_ng::from_str(&text)?)
    }

    /// Que el commit esté firmado por una clave de la allowlist.
    ///
    /// **No se inventa un formato de allowlist**: es el `allowed_signers` de ssh,
    /// que git ya consume por `gpg.ssh.allowedSignersFile`. Se le pasa por `-c` en
    /// vez de escribirlo en la config, porque verificar no configura nada.
    pub fn verify_signature(&self, commit: &str, signers: &Path) -> Result<()> {
        let out = Command::new("git")
            .args(["-C", &self.root.to_string_lossy()])
            .args(["-c", &format!("gpg.ssh.allowedSignersFile={}", signers.display())])
            .args(["verify-commit", commit])
            .output()?;
        if out.status.success() {
            return Ok(());
        }
        bail!(
            "sin firma de la allowlist ({})",
            first_line(&String::from_utf8_lossy(&out.stderr))
        )
    }

    /// Cuál de los dos árboles se movió entre dos commits: `(.bilink/, código)`.
    fn what_moved(&self, from: &str, to: &str) -> Result<(bool, bool)> {
        let diff = self.git(&["diff-tree", "-r", "--name-only", from, to])?;
        let mut bilink = false;
        let mut code = false;
        for path in diff.lines().filter(|l| !l.trim().is_empty()) {
            if is_bilink_path(path) { bilink = true } else { code = true }
        }
        Ok((bilink, code))
    }

    /// El árbol del commit: `read-tree` de `base`, más los `.bilink/` del árbol de
    /// trabajo.
    ///
    /// Los archivos se agregan **uno por uno y con `-f`**, no el directorio: `-f` es
    /// necesario porque `.bilink/` está en `info/exclude`, y enumerar es lo que
    /// impide que arrastre `cache/`, `index/` y `head`. Como el índice arranca del
    /// árbol del proyecto —que no tiene ningún `.bilink/`— las bajas salen gratis:
    /// lo que ya no está en disco tampoco entra.
    pub fn build_tree(&self, base: &str) -> Result<String> {
        let index = self.fresh_index(base)?;

        let files = self.tracked_bilink_files()?;
        for chunk in files.chunks(500) {
            let mut args = vec!["add".to_string(), "-f".into(), "--".into()];
            args.extend(chunk.iter().cloned());
            self.git_indexed_owned(&index, &args)?;
        }

        Ok(self.git_indexed(&index, &["write-tree"])?.trim().to_string())
    }

    /// El árbol de `base` más los `.bilink/` de **otro commit de la ref**, sin pasar
    /// por el árbol de trabajo.
    ///
    /// Es lo que `track` necesita: el árbol de código de la rama nueva y los bilinks
    /// heredados del commit del que se hereda. Ir por el árbol de trabajo obligaría
    /// a materializar antes de saber si el commit se puede escribir.
    pub fn build_tree_inheriting(&self, base: &str, bilinks_from: &str) -> Result<String> {
        let index = self.fresh_index(base)?;
        for (mode, oid, path) in self.bilink_entries(bilinks_from)? {
            self.git_indexed(
                &index,
                &["update-index", "--add", "--cacheinfo", &format!("{mode},{oid},{path}")],
            )?;
        }
        Ok(self.git_indexed(&index, &["write-tree"])?.trim().to_string())
    }

    /// Un índice propio recién leído de `base`.
    fn fresh_index(&self, base: &str) -> Result<PathBuf> {
        let index = self.index_path()?;
        if let Some(parent) = index.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&index);
        self.git_indexed(&index, &["read-tree", base])?;
        Ok(index)
    }

    /// Las entradas de `.bilink/` de un commit: `(mode, oid, path)`.
    fn bilink_entries(&self, commit: &str) -> Result<Vec<(String, String, String)>> {
        let out = self.git(&["ls-tree", "-r", commit])?;
        let mut entries = Vec::new();
        for line in out.lines() {
            let Some((meta, path)) = line.split_once('\t') else { continue };
            if !is_bilink_path(path) {
                continue;
            }
            let mut fields = meta.split_whitespace();
            let (Some(mode), Some("blob"), Some(oid)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            entries.push((mode.to_string(), oid.to_string(), path.to_string()));
        }
        Ok(entries)
    }

    /// Escribe un commit sobre la ref con padres y árbol dados, sin absorber nada.
    ///
    /// Es la puerta de `track`, el único comando cuyo commit no sale de la rama
    /// actual: sus dos padres vienen de lugares distintos.
    pub fn write_ref_commit(
        &self,
        branch: &str,
        tree: &str,
        parents: &[String],
        message: &str,
    ) -> Result<String> {
        let mut args = vec!["commit-tree".to_string(), tree.to_string()];
        for p in parents {
            args.push("-p".into());
            args.push(p.clone());
        }
        // **`commit-tree` no firma solo.** A diferencia de `git commit`, no mira
        // `commit.gpgsign`: hay que pasarle `-S`. Sin esto los commits de la ref
        // salen sin firmar y la allowlist del `pre-receive` no tendría nada que
        // verificar — la atestación de una decisión es el commit firmado, así que
        // sería la protección entera sin sujeto.
        //
        // La condición la pone git, no bilinker: la misma config con la que se
        // firma cualquier otro commit del repo, sin una opción propia que aprender.
        if self.signs() {
            args.push("-S".into());
        }
        args.push("-m".into());
        args.push(message.to_string());

        let sha = self.git_owned(&args)?.trim().to_string();
        self.git(&["update-ref", &Self::ref_name(branch), &sha])?;
        Ok(sha)
    }

    /// El índice propio, en `.git/bilink/index`.
    ///
    /// Dentro de `.git/` porque es por clon y no se versiona, y con nombre propio
    /// porque no es el índice del proyecto: el mismo `.bilink/` queda ignorado por
    /// uno y trackeado por el otro.
    pub fn index_path(&self) -> Result<PathBuf> {
        Ok(self.git_dir.join("bilink").join("index"))
    }

    /// Los archivos de `.bilink/` que sí se commitean, relativos a la raíz del repo.
    ///
    /// Recorre todas las capas: el exclude es por repo y un solo patrón las cubre a
    /// todas, estén donde estén.
    pub fn tracked_bilink_files(&self) -> Result<Vec<String>> {
        let mut out = BTreeSet::new();
        for dir in self.bilink_dirs()? {
            collect_tracked(&dir, &dir, &self.root, &mut out)?;
        }
        Ok(out.into_iter().collect())
    }

    /// Los `.bilink/` del árbol de trabajo, uno por capa.
    pub fn bilink_dirs(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        walk_for_bilink(&self.root, &mut out)?;
        out.sort();
        Ok(out)
    }

    // ─── `.bilink/head` ──────────────────────────────────────────────────────

    /// Escribe `head` en cada `.bilink/` del árbol.
    ///
    /// Lo escriben tanto la materialización como **todo commit sobre la ref**: si
    /// `accept` avanza la ref, el árbol pasa a corresponder al commit nuevo y `head`
    /// tiene que decirlo, o la guarda se dispararía después de cada aceptación.
    pub fn write_head(&self, branch: &str, ref_commit: &str) -> Result<()> {
        for dir in self.bilink_dirs()? {
            std::fs::write(
                dir.join("head"),
                format!("branch {branch}\ncommit {ref_commit}\n"),
            )
            .with_context(|| format!("escribiendo {}", dir.join("head").display()))?;
        }
        Ok(())
    }

    /// Lee `head` de la capa raíz. `None` cuando no hay procedencia.
    ///
    /// Un `.bilink/` sin `head` es un `.bilink/` del que no se sabe de dónde salió, y
    /// nadie lo pisa: es lo que hace que el paso 3 del corte pueda ser un `init` a
    /// secas.
    pub fn read_head(&self) -> Option<Head> {
        let dirs = self.bilink_dirs().ok()?;
        let text = std::fs::read_to_string(dirs.first()?.join("head")).ok()?;
        let mut branch = None;
        let mut commit = None;
        for line in text.lines() {
            match line.split_once(' ') {
                Some(("branch", v)) => branch = Some(v.trim().to_string()),
                Some(("commit", v)) => commit = Some(v.trim().to_string()),
                _ => {}
            }
        }
        Some(Head { branch: branch?, commit: commit? })
    }

    // ─── Materialización ─────────────────────────────────────────────────────

    /// Corrige el `.bilink/` del árbol si no corresponde a la rama actual.
    ///
    /// **Automático y sin ceremonia**: `git checkout` no toca `.bilink/` porque para
    /// el índice del proyecto son archivos ignorados, así que cambiar de rama deja
    /// el código de `B` con los bilinks de `A` y nada avisa. Cualquier comando
    /// compara `head` contra la rama actual y, si no coinciden, materializa y sigue
    /// — no hay comando de más que tipear ni pregunta que contestar.
    ///
    /// Con `HEAD` desacoplado no se materializa nada: no hay rama contra la cual
    /// comparar, y adivinar una sería peor.
    pub fn ensure_current(&self) -> Result<Materialization> {
        let Some(branch) = self.branch() else {
            return Ok(Materialization::Detached);
        };
        let head = self.read_head();
        let tip = match self.ref_tip(&branch) {
            Some(t) => t,
            None => return Ok(Materialization::NoRef(branch)),
        };

        match head {
            Some(h) if h.branch == branch && h.commit == tip => Ok(Materialization::UpToDate),
            None => Ok(Materialization::NoProvenance),
            Some(h) => {
                self.guard_clean(&h)?;
                self.materialize(&branch, &tip)?;
                Ok(Materialization::Rematerialized { from: h, to: tip })
            }
        }
    }

    /// **La guarda, que es una sola: la de git.**
    ///
    /// Si el `.bilink/` del árbol difiere del commit que `head` nombra, hay trabajo
    /// que no está en ninguna parte —`.bilink/` está fuera del git del proyecto— y
    /// materializar lo destruiría. Ahí se para, igual que `git checkout` se niega a
    /// pisar cambios.
    ///
    /// En el flujo diseñado esa ventana no existe: `accept` y `apply` commitean sobre
    /// la ref como parte del acto. Queda abierta sólo para un `apply` que crashea a
    /// mitad o una edición a mano, y las dos merecen un humano.
    pub fn guard_clean(&self, head: &Head) -> Result<()> {
        let dirty = self.dirty_against(&head.commit)?;
        if dirty.is_empty() {
            return Ok(());
        }
        bail!(
            "el .bilink/ del árbol difiere de {} ({}{})\n\n  \
             Ese trabajo no está en ninguna otra parte: .bilink/ está fuera del git\n  \
             del proyecto. No se materializó nada.",
            short(&head.commit),
            dirty.iter().take(3).cloned().collect::<Vec<_>>().join(", "),
            if dirty.len() > 3 { format!(", +{}", dirty.len() - 3) } else { String::new() }
        )
    }

    /// Los paths de `.bilink/` en que el árbol de trabajo difiere de un commit de la
    /// ref. Se calcula contra el índice propio, no contra el del proyecto.
    pub fn dirty_against(&self, ref_commit: &str) -> Result<Vec<String>> {
        let want = self.bilink_blobs(ref_commit)?;
        let have = self.tracked_bilink_files()?;

        let mut dirty = Vec::new();
        for path in &have {
            match want.iter().find(|(p, _)| p == path) {
                Some((_, oid)) => {
                    let actual = self.hash_object(path)?;
                    if &actual != oid {
                        dirty.push(path.clone());
                    }
                }
                None => dirty.push(path.clone()),
            }
        }
        for (path, _) in &want {
            if !have.contains(path) {
                dirty.push(path.clone());
            }
        }
        dirty.sort();
        dirty.dedup();
        Ok(dirty)
    }

    /// Escribe en el árbol el `.bilink/` de un commit de la ref, y deja `head`.
    ///
    /// Sólo toca paths de `.bilink/`: el código sale de la rama del proyecto, que ya
    /// está checkouteada. Los archivos que la ref no tiene se borran, porque un
    /// bilink removido tiene que desaparecer del árbol igual que un archivo que un
    /// `git checkout` saca.
    pub fn materialize(&self, branch: &str, ref_commit: &str) -> Result<usize> {
        let want = self.bilink_blobs(ref_commit)?;
        let have = self.tracked_bilink_files()?;

        for path in &have {
            if !want.iter().any(|(p, _)| p == path) {
                let _ = std::fs::remove_file(self.root.join(path));
            }
        }
        for (path, oid) in &want {
            let full = self.root.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let blob = self.git(&["cat-file", "blob", oid])?;
            std::fs::write(&full, blob)
                .with_context(|| format!("escribiendo {}", full.display()))?;
        }

        self.write_gitignore()?;
        self.write_head(branch, ref_commit)?;
        Ok(want.len())
    }

    /// La rama del proyecto que un nombre escrito a mano nombra.
    ///
    /// `origin/main` y `main` son la misma rama, y la ref es una sola. Pero
    /// `feature/x` **también** lleva una barra, así que partir por la última no
    /// sirve: dejaría `x`. El prefijo se saca sólo si es el nombre de un remoto de
    /// este repo, que es la única forma de distinguir los dos casos.
    pub fn resolve_branch_name(&self, name: &str) -> String {
        if self.ref_tip(name).is_some() {
            return name.to_string();
        }
        for remote in crate::config::remotes(&self.root).unwrap_or_default() {
            if let Some(rest) = name.strip_prefix(&format!("{remote}/")) {
                return rest.to_string();
            }
        }
        name.to_string()
    }

    /// Como [`Self::git`], pero devuelve stdout aunque git salga con error.
    ///
    /// Para los comandos cuyo código de salida no significa falla: `diff --no-index`
    /// sale con 1 cuando los archivos difieren, que es el caso que interesa.
    pub fn git_lenient(&self, args: &[&str]) -> String {
        Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    /// Si el repo tiene alguna `refs/bilink/*`. Distingue un repo que ya cortó de
    /// uno que todavía lleva los bilinks en la rama.
    pub fn has_any_ref(&self) -> Result<bool> {
        Ok(!self.git(&["for-each-ref", "--format=%(refname)", "refs/bilink/"])?.trim().is_empty())
    }

    /// La base de merge entre dos commits, o `None` si no la tienen.
    ///
    /// Entre dos refs de bilinks sale gratis: es la base real, porque `track` pone
    /// el commit del que hereda como **primer padre** en vez de copiar archivos.
    pub fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        Ok(self.git(&["merge-base", a, b]).ok().map(|s| s.trim().to_string()))
    }

    /// Los paths de `.bilink/` de un commit.
    pub fn bilink_paths_in(&self, commit: &str) -> Result<Vec<String>> {
        Ok(self.bilink_blobs(commit)?.into_iter().map(|(p, _)| p).collect())
    }

    /// Los blobs de `.bilink/` de un commit de la ref: `(path, oid)`.
    fn bilink_blobs(&self, ref_commit: &str) -> Result<Vec<(String, String)>> {
        let out = self.git(&["ls-tree", "-r", ref_commit])?;
        let mut blobs = Vec::new();
        for line in out.lines() {
            let Some((meta, path)) = line.split_once('\t') else { continue };
            if !is_bilink_path(path) {
                continue;
            }
            let mut fields = meta.split_whitespace();
            let (_mode, kind, oid) = (fields.next(), fields.next(), fields.next());
            if kind != Some("blob") {
                continue;
            }
            if let Some(oid) = oid {
                blobs.push((path.to_string(), oid.to_string()));
            }
        }
        Ok(blobs)
    }

    /// `.bilink/.gitignore` con `cache/` e `index/`, en cada capa.
    ///
    /// Adentro de `.bilink/` y no en `info/exclude`: adentro viaja con el directorio
    /// que gobierna, así que una capa nueva en cualquier repo trae su regla puesta.
    fn write_gitignore(&self) -> Result<()> {
        for dir in self.bilink_dirs()? {
            let path = dir.join(".gitignore");
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            let mut out = existing.clone();
            for entry in ["cache/", "index/"] {
                if !existing.lines().any(|l| l.trim() == entry) {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str(entry);
                    out.push('\n');
                }
            }
            if out != existing {
                std::fs::write(&path, out)?;
            }
        }
        Ok(())
    }

    fn hash_object(&self, path: &str) -> Result<String> {
        Ok(self.git(&["hash-object", "--", path])?.trim().to_string())
    }

    // ─── Utilidades de git ───────────────────────────────────────────────────

    fn rev_parse(&self, rev: &str) -> Option<String> {
        let out = self.git(&["rev-parse", "--verify", "--quiet", rev]).ok()?;
        let sha = out.trim();
        (!sha.is_empty()).then(|| sha.to_string())
    }

    pub fn git(&self, args: &[&str]) -> Result<String> {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        self.git_owned(&owned)
    }

    fn git_owned(&self, args: &[String]) -> Result<String> {
        run(Command::new("git").args(args).current_dir(&self.root), args)
    }

    fn git_indexed(&self, index: &Path, args: &[&str]) -> Result<String> {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        self.git_indexed_owned(index, &owned)
    }

    fn git_indexed_owned(&self, index: &Path, args: &[String]) -> Result<String> {
        run(
            Command::new("git").args(args).current_dir(&self.root).env("GIT_INDEX_FILE", index),
            args,
        )
    }
}

/// A qué rama y a qué commit de la ref corresponde el `.bilink/` del árbol.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    pub branch: String,
    pub commit: String,
}

/// La absorción que precede a un acto, si falta.
///
/// **Absorber es precondición de todo commit sobre la ref, y se cumple en un commit
/// propio.** Llamarla N veces seguidas absorbe una sola: la segunda encuentra el tip
/// ya absorbido y devuelve `None`. Es lo que hace que las N decisiones de un
/// `accept .` cuelguen todas de la misma absorción sin que nadie lleve la cuenta.
pub fn absorb_act(dir: &Path) -> Result<Option<Commit>> {
    let Some((repo, branch)) = committable(dir)? else { return Ok(None) };
    repo.absorb(&branch)
}

/// El commit de decisión que cierra una aceptación o un `apply`.
///
/// **La granularidad sigue al objeto, no al acto**: una aceptación, no una
/// invocación. `accept .` sobre veinte endpoints pasa por acá veinte veces, y cada
/// commit lleva su propio endpoint — cien commits firmados denuncian una aprobación
/// masiva que uno disimula.
pub fn decide_act(dir: &Path, message: &RefMessage) -> Result<Option<Commit>> {
    let Some((repo, branch)) = committable(dir)? else { return Ok(None) };
    Ok(Some(repo.decide(&branch, message)?))
}

/// El repo y la rama sobre la que se commitea, o `None` si no hay dónde.
///
/// **`None` cuando la rama no tiene ref.** Es el estado de un repo antes del corte
/// `005`: los bilinks todavía viven en la rama, git los ve como siempre, y
/// commitearlos es de quien trabaja. Que el corte sea lo que enciende esto es lo que
/// permite que el binario nuevo corra sobre repos que todavía no cortaron.
fn committable(dir: &Path) -> Result<Option<(Repo, String)>> {
    let repo = Repo::open(dir)?;

    let Some(branch) = repo.branch() else {
        // En `HEAD` desacoplado los comandos que commitean sobre la ref se niegan —
        // pero sólo si este repo ya está en la ref.
        if repo.has_any_ref()? {
            bail!(
                "HEAD está desacoplado: no se puede commitear sobre la ref.\n  \
                 Volver a una rama; lo que se escribió en .bilink/ sigue en el árbol."
            );
        }
        return Ok(None);
    };

    if repo.ref_tip(&branch).is_none() {
        return Ok(None);
    }
    Ok(Some((repo, branch)))
}

/// Qué pasó con el `.bilink/` del árbol al empezar un comando.
#[derive(Debug, Clone, PartialEq)]
pub enum Materialization {
    /// El árbol ya corresponde a la rama actual.
    UpToDate,
    /// Se materializó el `.bilink/` de la ref correcta.
    Rematerialized { from: Head, to: String },
    /// Hay `.bilink/` en el árbol y no hay `head`: no se sabe de dónde salió y no se
    /// pisa. Es el estado del paso 3 del corte.
    NoProvenance,
    /// La rama no tiene ref todavía. El arreglo es `track`, no materializar.
    NoRef(String),
    /// `HEAD` desacoplado: los comandos de lectura corren contra lo que `head` dice,
    /// avisando; los que commitean se niegan.
    Detached,
    /// No hay repo git. bilinker corre igual: la raíz cae al cwd, que es lo que
    /// permite usarlo en un proyecto nuevo sin ningún paso de inicialización.
    NoGit,
}

/// Qué hace un commit sobre la ref. Uno de tres, nunca dos.
///
/// Lo que los separa es de git —padres y árboles—, no de bilinker: `absorb` y
/// `decide` los escriben con esta forma, y [`Repo::classify`] la lee de vuelta sin
/// abrir un solo bilink.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// El corte. Padre único, y del proyecto: es el único commit de la ref que no
    /// tiene ninguna absorción debajo, y la fidelidad se lee contra ese padre.
    Cut { project: String },
    /// **Trae código.** Dos padres, el segundo del proyecto, `.bilink/` sin tocar.
    Absorption { project: String },
    /// **Decide.** Un padre, el árbol de código sin cambios.
    Decision,
    /// **Sincroniza decisiones.** Dos padres, los dos de la ref, el árbol de código
    /// sin cambios. Es lo que escribe `adopt`.
    Synchronization,
}

/// El resultado de commitear sobre la ref.
#[derive(Debug, Clone)]
pub struct Commit {
    pub sha: String,
    /// El commit del proyecto absorbido, o `None` si ya lo estaba — el acto tiene un
    /// solo padre y su árbol de código no cambió.
    pub absorbed: Option<String>,
    /// `false` cuando no había nada que escribir.
    pub wrote: bool,
}

/// Si un path del repo cae dentro de algún `.bilink/`.
fn is_bilink_path(path: &str) -> bool {
    path.split('/').any(|c| c == ".bilink")
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()
}

fn run(cmd: &mut Command, args: &[String]) -> Result<String> {
    let out = cmd
        .output()
        .with_context(|| format!("corriendo git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} falló: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Los `.bilink/` de **este** repo, recorriendo desde su raíz.
///
/// **Se para en la frontera del repo.** Un subdirectorio que tiene su propio `.git`
/// es otro repositorio, y sus bilinks son suyos: la ref es por repo, y absorberlos
/// acá los metería en un snapshot que no los describe — el árbol de código del
/// commit absorbido no los contiene, así que ni la disyunción ni la fidelidad
/// hablarían de ellos.
///
/// No es hipotético: en accreta cada subsistema tiene su capa de implementación en
/// un repo propio, gitignoreado por el padre, y sin este freno el corte del padre se
/// tragaba los bilinks de los tres.
fn walk_for_bilink(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let candidate = dir.join(".bilink");
    if candidate.is_dir() {
        out.push(candidate);
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ".git" || name == ".bilink" || name == "target" || name.starts_with(".bilink-migrate-") {
            continue;
        }
        // La frontera del repo. `.git` es un directorio en un clon normal y un
        // archivo en un worktree lincado; las dos formas cuentan.
        if path.join(".git").exists() {
            continue;
        }
        walk_for_bilink(&path, out)?;
    }
    Ok(())
}

/// Los archivos de un `.bilink/` que sí van al commit de la ref.
///
/// Se excluyen dos cosas, y por razones distintas:
///
/// - [`NOT_COMMITTED`] — derivados y estado del árbol.
/// - **Los clones de proveedores**, que son otros repos enteros viviendo en
///   `.bilink/<alias>/`. Un clon ajeno no es contenido de esta capa: se trae, se
///   descarta y se vuelve a traer, y su procedencia es su propio remoto. La regla
///   es la misma que frena el recorrido de capas —un directorio con `.git` adentro
///   es otro repositorio— y acá se aplica al mismo hecho por el mismo motivo.
///
/// Que hasta ahora no se filtraran fue suerte: git trata un repo anidado como
/// frontera por su cuenta. Depender de eso es depender de que el clon esté sano.
fn collect_tracked(dir: &Path, base: &Path, root: &Path, out: &mut BTreeSet<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let rel_to_base = path.strip_prefix(base).unwrap_or(&path);
        if rel_to_base
            .components()
            .next()
            .map(|c| NOT_COMMITTED.contains(&c.as_os_str().to_string_lossy().as_ref()))
            .unwrap_or(false)
        {
            continue;
        }
        if path.is_dir() {
            // La frontera del repo, dicha una vez más: adentro de `.bilink/` un
            // directorio con `.git` es el clon de un proveedor.
            if path.join(".git").exists() {
                continue;
            }
            collect_tracked(&path, base, root, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.insert(rel.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un repo con la ref ya cortada: `X` saca `.bilink/` del índice, `●0` lo trae de
    /// vuelta sobre la ref. Devuelve `(tmp, repo, branch, X, ●0)`.
    fn cut_repo() -> (tempfile::TempDir, Repo, String, String, String) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let g = |args: &[&str]| {
            let out = Command::new("git").current_dir(&root).args(args).output().expect("git");
            assert!(out.status.success(), "git {args:?}: {}",
                    String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);

        // La exclusión que pone `init`: sin ella un `add -A` del proyecto se llevaría
        // `.bilink/` a la rama y rompería la disyunción.
        std::fs::write(root.join(".git/info/exclude"), ".bilink/\n").unwrap();

        std::fs::write(root.join("code.txt"), "uno\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "código"]);

        // `X`: el commit del proyecto sin `.bilink/` en el árbol.
        let x = g(&["rev-parse", "HEAD"]);

        // `●0`: el corte, con `X` como padre único y `.bilink/` en el árbol.
        std::fs::create_dir_all(root.join(".bilink")).unwrap();
        std::fs::write(root.join(".bilink/a.yaml"), "endpoint: {}\n").unwrap();
        let blob = g(&["hash-object", "-w", ".bilink/a.yaml"]);
        let tree = tree_with(&root, &x, &[(".bilink/a.yaml", &blob)]);
        let cut = g(&["commit-tree", &tree, "-p", &x, "-m", "corte"]);
        g(&["update-ref", "refs/bilink/main", &cut]);

        let repo = Repo::open(&root).expect("abrir el repo");
        (tmp, repo, "main".to_string(), x, cut)
    }

    /// El árbol de `base` más los blobs dados bajo `.bilink/`, con un índice aparte.
    fn tree_with(root: &Path, base: &str, files: &[(&str, &str)]) -> String {
        let index = root.join(".git/test-index");
        let _ = std::fs::remove_file(&index);
        let g = |args: &[&str]| {
            let out = Command::new("git")
                .current_dir(root)
                .env("GIT_INDEX_FILE", &index)
                .args(args)
                .output()
                .expect("git");
            assert!(out.status.success(), "git {args:?}: {}",
                    String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        g(&["read-tree", base]);
        for (path, blob) in files {
            g(&["update-index", "--add", "--cacheinfo", &format!("100644,{blob},{path}")]);
        }
        g(&["write-tree"])
    }

    /// Un mensaje de decisión cualquiera, para los tests que no miran el mensaje.
    fn decision() -> RefMessage {
        RefMessage::new(RefCommand::Accept {
            place: true,
            content: true,
            uuid: "00000000-0000-4000-8000-000000000000".into(),
            n: 0,
        })
    }

    fn git_in(root: &Path, args: &[&str]) -> String {
        let out = Command::new("git").current_dir(root).args(args).output().expect("git");
        assert!(out.status.success(), "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// El corte se reconoce solo: padre único, y del proyecto.
    #[test]
    fn the_cut_is_classified_as_the_cut() {
        let (_t, repo, _b, x, cut) = cut_repo();
        assert_eq!(repo.classify(&cut).unwrap(), Act::Cut { project: x });
    }

    /// Una absorción: dos padres, el segundo del proyecto, `.bilink/` sin tocar.
    #[test]
    fn an_absorption_is_classified_as_one() {
        let (_t, repo, branch, _x, _cut) = cut_repo();
        std::fs::write(repo.root.join("code.txt"), "dos\n").unwrap();
        git_in(&repo.root, &["add", "-A"]);
        git_in(&repo.root, &["commit", "-qm", "el código avanza"]);
        let e = git_in(&repo.root, &["rev-parse", "HEAD"]);

        let c = repo.absorb(&branch).unwrap().expect("había algo que absorber");
        assert_eq!(repo.classify(&c.sha).unwrap(), Act::Absorption { project: e });
    }

    /// Una decisión: un padre, y el árbol de código de la absorción que tiene arriba.
    #[test]
    fn a_decision_is_classified_as_one() {
        let (_t, repo, branch, _x, _cut) = cut_repo();
        std::fs::write(repo.root.join(".bilink/a.yaml"), "endpoint: {b: 1}\n").unwrap();

        let c = repo.decide(&branch, &decision()).unwrap();
        assert!(c.wrote);
        assert_eq!(repo.classify(&c.sha).unwrap(), Act::Decision);
    }

    /// **El negativo.** Un commit que absorbe y decide a la vez se rechaza: es la
    /// invariante 4, y es lo que un `pre-receive` va a chequear con git a secas.
    #[test]
    fn a_commit_that_absorbs_and_decides_at_once_is_rejected() {
        let (_t, repo, branch, _x, cut) = cut_repo();
        let root = repo.root.clone();

        std::fs::write(root.join("code.txt"), "dos\n").unwrap();
        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-qm", "el código avanza"]);
        let e = git_in(&root, &["rev-parse", "HEAD"]);

        // A mano, la forma vieja: un merge que trae `e` **y** escribe una decisión.
        std::fs::write(root.join(".bilink/a.yaml"), "endpoint: {b: 1}\n").unwrap();
        let blob = git_in(&root, &["hash-object", "-w", ".bilink/a.yaml"]);
        let tree = tree_with(&root, &e, &[(".bilink/a.yaml", &blob)]);
        let sha = git_in(&root, &["commit-tree", &tree, "-p", &cut, "-p", &e,
                                  "-m", "accept 0000.0"]);

        let err = repo.classify(&sha).unwrap_err().to_string();
        assert!(err.contains("absorbe y decide"), "y se dice por qué:\n{err}");

        // Y las dos puertas juntas escriben lo mismo en dos commits, los dos válidos.
        let a = repo.absorb(&branch).unwrap().expect("había algo que absorber");
        let d = repo.decide(&branch, &decision()).unwrap();
        assert_eq!(repo.classify(&a.sha).unwrap(), Act::Absorption { project: e });
        assert_eq!(repo.classify(&d.sha).unwrap(), Act::Decision);
        assert_eq!(rev_tree(&root, &d.sha), rev_tree(&root, &sha),
                   "el mismo árbol resultante, en dos commits en vez de uno");
    }

    fn rev_tree(root: &Path, commit: &str) -> String {
        git_in(root, &["rev-parse", &format!("{commit}^{{tree}}")])
    }

    /// Una decisión **no absorbe de contrabando**: con el tip del proyecto sin
    /// absorber, `decide` falla en vez de escribir un merge.
    #[test]
    fn decide_refuses_to_absorb_on_its_own() {
        let (_t, repo, branch, _x, _cut) = cut_repo();
        std::fs::write(repo.root.join("code.txt"), "dos\n").unwrap();
        git_in(&repo.root, &["add", "-A"]);
        git_in(&repo.root, &["commit", "-qm", "el código avanza"]);

        let before = repo.ref_tip(&branch).unwrap();
        let err = repo.decide(&branch, &decision()).unwrap_err().to_string();
        assert!(err.contains("no está absorbido"), "y se dice por qué:\n{err}");
        assert_eq!(before, repo.ref_tip(&branch).unwrap(), "no se escribió nada");
    }

    /// Absorber dos veces seguidas absorbe **una**: es lo que hace que las N
    /// decisiones de un `accept .` cuelguen todas del mismo merge.
    #[test]
    fn absorbing_twice_in_a_row_absorbs_once() {
        let (_t, repo, branch, _x, _cut) = cut_repo();
        std::fs::write(repo.root.join("code.txt"), "dos\n").unwrap();
        git_in(&repo.root, &["add", "-A"]);
        git_in(&repo.root, &["commit", "-qm", "el código avanza"]);

        assert!(repo.absorb(&branch).unwrap().is_some());
        assert!(repo.absorb(&branch).unwrap().is_none(), "el tip ya estaba absorbido");
    }
}
