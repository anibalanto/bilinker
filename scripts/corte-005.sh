#!/usr/bin/env bash
#
# corte-005 — mover los bilinks de un repo a refs/bilink/<branch>.
#
# Es el runbook de la task `e` y su test a la vez: los pasos que ejecuta son los
# que el ADR-0004 § `005` prescribe, y las verificaciones que corre son las que
# hacen que el corte sea revisable en vez de creído.
#
#   1. UN commit que saca .bilink/ del índice de la rama   → X
#   2. bilinker init                                       (exclude + refspec)
#   3. bilinker track <branch>                             → ●0, padre X
#   4. Ledger: 005
#
# El paso 1 es `git rm --cached` más commit: los archivos se quedan en disco, así
# que no hay que restaurarlos para commitearlos a la ref.
#
# Uso:  corte-005.sh <repo> [--dry-run]
#
# Es por repo, no por capa: un solo patrón `.bilink/` en el exclude cubre todas
# las capas de ese repo, y un solo commit de la ref las lleva a todas.

set -euo pipefail

REPO="${1:?uso: corte-005.sh <repo> [--dry-run]}"
DRY_RUN="${2:-}"

cd "$REPO"
REPO="$(git rev-parse --show-toplevel)"
cd "$REPO"

BRANCH="$(git symbolic-ref --short HEAD)"
REF="refs/bilink/$BRANCH"
LEDGER=".accreta/migrations"

say()  { printf '%s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

# ─── Precondiciones ────────────────────────────────────────────────────────

[ -d .bilink ] || fail "no hay .bilink/ en $REPO — nada que cortar"

git rev-parse --verify --quiet "$REF" >/dev/null \
  && fail "$REF ya existe: este repo ya cortó. Para ponerla al día, \`bilinker sync\`."

[ -z "$(git status --porcelain)" ] \
  || fail "el árbol tiene cambios sin commitear. El corte parte de un árbol limpio."

# Las capas de **este** repo: el recorrido se para en la frontera, porque un
# subdirectorio con su propio .git es otro repositorio y sus bilinks son suyos.
LAYERS=()
while IFS= read -r d; do LAYERS+=("$d"); done < <(
  find . -name .bilink -type d -not -path '*/target/*' -printf '%h\n' \
  | while read -r parent; do
      [ "$parent" = "." ] || [ ! -e "$parent/.git" ] && printf '%s\n' "$parent"
    done | sort -u
)
[ "${#LAYERS[@]}" -gt 0 ] || fail "no se encontró ninguna capa"

say "repo:    $REPO"
say "rama:    $BRANCH"
say "capas:   ${LAYERS[*]}"

if [ "$DRY_RUN" = "--dry-run" ]; then
  say
  say "dry-run: no se escribió nada"
  exit 0
fi

# ─── 1. El commit que saca .bilink/ del índice ─────────────────────────────

for layer in "${LAYERS[@]}"; do
  git ls-files --error-unmatch "$layer/.bilink" >/dev/null 2>&1 \
    && git rm --cached -r -q "$layer/.bilink"
done

# El ledger, en el mismo commit: es el que dice "este repo movió sus bilinks a la
# ref", así que la línea que lo registra pertenece acá y no a un paso aparte.
mkdir -p "$(dirname "$LEDGER")"
grep -qx "bilinker-005-ref-cutover" "$LEDGER" 2>/dev/null \
  || printf 'bilinker-005-ref-cutover\n' >> "$LEDGER"
git add "$LEDGER"

git diff --cached --quiet && fail "el paso 1 no tenía nada que commitear"
git commit -q -m "corte 005: los bilinks salen de la rama y pasan a $REF"
X="$(git rev-parse HEAD)"
say "paso 1:  X = ${X:0:7}"

# ─── 2. init: exclude y refspec ────────────────────────────────────────────
#
# No materializa nada: hay .bilink/ en el árbol y no hay head, así que init no
# puede saber de dónde salió y lo deja intacto. Es lo que hace que el corte no
# necesite setup propio.

bilinker init >/dev/null
say "paso 2:  exclude + refspec"

# ─── 3. track: el commit que crea la ref ───────────────────────────────────
#
# Ningún commit de refs/bilink/* califica como herencia, así que la ref nace
# desde cero con el .bilink/ del árbol y X como padre único. El corte no es un
# camino aparte: es el caso "no hay de quién heredar" de track.

bilinker track "$BRANCH" >/dev/null
ZERO="$(git rev-parse "$REF")"
say "paso 3:  ●0 = ${ZERO:0:7}"

# ─── Verificaciones ────────────────────────────────────────────────────────
#
# Lo que las hace valer la pena es que corren sobre el resultado, no sobre la
# intención. Cada una es la invariante dicha como comando.

# ●0 nace de X como padre único.
PARENTS="$(git rev-list --parents -n 1 "$REF" | cut -d' ' -f2-)"
[ "$PARENTS" = "$X" ] || fail "●0 debería tener a X como padre único, y tiene: $PARENTS"

# Disyunción: ninguna rama del proyecto contiene .bilink/.
git ls-tree -r --name-only HEAD | grep -q '\.bilink/' \
  && fail "la rama del proyecto todavía contiene .bilink/"

# Fidelidad: el árbol de código de ●0 es idéntico al de X. Todo lo que difiere
# cae dentro de algún .bilink/.
STRAYS="$(git diff-tree -r --name-only "$X" "$REF" | grep -v '\.bilink/' || true)"
[ -z "$STRAYS" ] || fail "el árbol de código de ●0 no es el de X: $STRAYS"

# Los derivados quedan fuera del commit.
DERIVED="$(git ls-tree -r --name-only "$REF" | grep -E '\.bilink/(cache|index)/|\.bilink/head$' || true)"
[ -z "$DERIVED" ] || fail "la ref se llevó derivados: $DERIVED"

# La ref no es una rama.
git branch -a | grep -q 'bilink' && fail "la ref aparece en git branch -a"

# Y los bilinks siguen en el árbol de trabajo, donde check los necesita.
for layer in "${LAYERS[@]}"; do
  [ -d "$layer/.bilink" ] || fail "$layer/.bilink desapareció del árbol"
done

# Ningún .bilink/ de un repo anidado entró.
NESTED="$(git ls-tree -r --name-only "$REF" | grep '\.bilink/' \
  | while read -r f; do
      d="$f"; while [ "$d" != "." ] && [ "$d" != "/" ]; do
        d="$(dirname "$d")"
        [ "$d" = "." ] && break
        [ -e "$d/.git" ] && printf '%s\n' "$f" && break
      done
    done || true)"
[ -z "$NESTED" ] || fail "la ref se tragó bilinks de un repo anidado: $NESTED"

say "verif:   disyunción · fidelidad · derivados fuera · ref no es rama · frontera"
say
say "cortado. Falta pushear: git push && git push origin '$REF:$REF'"
