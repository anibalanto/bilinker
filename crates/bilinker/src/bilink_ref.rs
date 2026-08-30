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
//!   sobre la ref**, y cuando no se cumple se cumple absorbiendo en el mismo commit.
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

/// Lo que queda fuera del índice de bilinker, no sólo del índice del proyecto.
///
/// `cache/` e `index/` son derivados; `head` es estado del árbol de trabajo, no
/// contenido. Ninguno se commitea.
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

    /// Escribe un commit sobre `refs/bilink/<branch>`, absorbiendo si hace falta.
    ///
    /// ```text
    /// read-tree     <commit del proyecto absorbido>   ← el nuevo, si hay que absorber;
    /// update-index  únicamente .bilink/                 el vigente, si ya está absorbido
    /// ```
    ///
    /// Nada del árbol de trabajo fuera de `.bilink/` entra jamás. Cuando el proyecto
    /// no se movió desde la última absorción el acto es un commit común de un solo
    /// padre: **el merge no es la forma del acto, es la forma de ponerse al día.**
    pub fn commit(&self, branch: &str, message: &str) -> Result<Commit> {
        let ref_tip = self.require_ref_tip(branch)?;
        let project_tip = self.branch_tip(branch)?;
        let absorbed = self.absorbed(&ref_tip)?.unwrap_or_else(|| ref_tip.clone());

        let absorbing = absorbed != project_tip;
        let base = if absorbing { &project_tip } else { &absorbed };

        if absorbing {
            self.verify_disjoint(&project_tip)?;
        }

        let tree = self.build_tree(base)?;
        self.verify_faithful(&tree, base)?;

        let previous_tree = self.git(&["rev-parse", &format!("{ref_tip}^{{tree}}")])?;
        if tree.trim() == previous_tree.trim() && !absorbing {
            return Ok(Commit { sha: ref_tip, absorbed: None, wrote: false });
        }

        let mut args = vec!["commit-tree".to_string(), tree.clone(), "-p".into(), ref_tip];
        if absorbing {
            args.push("-p".into());
            args.push(project_tip.clone());
        }
        args.push("-m".into());
        args.push(message.to_string());

        let sha = self.git_owned(&args)?.trim().to_string();
        self.git(&["update-ref", &Self::ref_name(branch), &sha])?;
        self.write_head(branch, &sha)?;

        Ok(Commit {
            sha,
            absorbed: absorbing.then_some(project_tip),
            wrote: true,
        })
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

/// El commit de la ref que cierra un acto.
///
/// **La granularidad sigue al acto, no al objeto**: una invocación de `accept` o de
/// `apply`, no una aceptación. `accept .` da un commit, con el mensaje enumerando
/// los endpoints, porque es una persona mirando y decidiendo **una vez**, y partirlo
/// en N commits firmados no agrega verdad.
///
/// **No hace nada cuando la rama no tiene ref.** Es el estado de un repo antes del
/// corte `005`: los bilinks todavía viven en la rama, git los ve como siempre, y
/// commitearlos es de quien trabaja. Que el corte sea lo que enciende esto es lo que
/// permite que el binario nuevo corra sobre repos que todavía no cortaron.
pub fn commit_act(dir: &Path, message: &str) -> Result<Option<Commit>> {
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
    Ok(Some(repo.commit(&branch, message)?))
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
}

/// El resultado de commitear sobre la ref.
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
        walk_for_bilink(&path, out)?;
    }
    Ok(())
}

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
            collect_tracked(&path, base, root, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.insert(rel.to_string_lossy().into_owned());
        }
    }
    Ok(())
}
