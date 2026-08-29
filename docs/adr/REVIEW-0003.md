# Pedido de revisión — ADR-0003

Revisión adversarial de un ADR antes de aceptarlo. No lo reescribas: reportá hallazgos.

## Archivos

- ADR: `subsystems/bilinker/.stratum/impl/docs/adr/0003-captures-inmutables-y-bilinks-abstractos.md`
- Escenarios: mismo directorio, `...-scenarios.yaml`
- ADR previo, para contraste: `0002-bilinker-universal-structural-references.md`

Está en español, en estado Propuesto, con 7 decisiones y ~46 escenarios. Propone cambios grandes al formato de bilinker: captures inmutables direccionados por contenido, partición en cuatro archivos, eliminación de `resolved_at`/`kind`/`name.N`, un endpoint `abstract`, un endpoint hacia otro repo, mover los bilinks a `refs/bilink/<branch>`, y una migración por etapas con carpetas `.bilink-migrate-<id>`.

## Qué revisar, en este orden

**1. Justificaciones huérfanas.** Es el modo de falla dominante de este documento y **ya se dio dos veces**: la Decisión 6 llegó a afirmar que un `check` dejaba un commit, cuando la Decisión 5 había sacado el estado de git; y la Decisión 7 justificaba su orden con una premisa que la Decisión 6 dejó de cumplir. Las dos son lo mismo — texto que argumenta apoyándose en algo que otra decisión cambió después.

Buscá específicamente eso: para cada afirmación que sostiene una decisión, verificá que su premisa siga siendo cierta en el documento *actual*. Es un patrón rastreable, no una consigna general. Se corrigieron esas dos; asumí que hay más.

**2. Verificá cada afirmación sobre el repo.** El ADR asegura cosas sobre las specs y el código; NO le creas. Contrastá contra:

- specs: `subsystems/bilinker/{concepts,commands,scenarios}/`, `subsystems/stratum/concepts/`, `subsystems/lattice/concepts/`, `concepts/migration.md`
- código: `subsystems/bilinker/.stratum/impl/crates/bilinker/src/`
- skill: `ia/skills/bilinker/SKILL.md`

Interesan sobre todo: que `check.rs` copie el hash estructural del vecino y no el del archivo; que `KEYS` en `bilink.rs` no incluya `kind` ni `name.N`; que `LinkEndpoint` no tenga variante `Bilink`; que `grammar.rs` soporte YAML/Markdown/TS; que las invariantes que cita (`capture.md` inv. 4, `bilink.md` inv. 2/6/7/13, `migration.md` inv. 3/5) digan lo que el ADR dice; y que el nodo canónico de lattice dependa del `range` del capture como el ADR afirma.

**3. Solidez del diseño.** ¿Los argumentos se sostienen? ¿Hay consecuencias no previstas? Prestá atención a si alguna decisión rompe una invariante que el ADR declara respetar. Zonas con más superficie: la coexistencia durante la migración (Decisión 7), y la asimetría local/remoto de la Decisión 6 — qué pasa si la ref queda desincronizada.

**Un punto merece escrutinio extra: la eliminación de `resolved_at`.** Es la decisión más joven del ADR y la única que no pasó por varias vueltas de discusión — se tomó en un solo intercambio. El argumento es que `commit.N` domina su único uso funcional. Verificalo vos: ¿`commit.N` está siempre presente cuando se lo necesitaría?, ¿qué pasa con un endpoint nunca aceptado?, ¿el reemplazo de `git log --since=<resolved_at>` por `git log <commit.N>..HEAD` da el mismo resultado en todos los casos que las tablas de "Fuente del cambio" cubren? Y si la eliminación se sostiene, chequeá que el ADR declare todo lo que arrastra: además de la invariante 13 de `bilink.md`, hay que reespecificar esas tablas en `consistency.md` y `check.md`, y el ADR podría no decirlo con suficiente detalle.

## Contexto útil

Verificado a mano y probablemente sólido: el `hash.1` del nodo spec de `b95021d2` en `subsystems/stratum/.bilink/` es el `hash.1` estructural del nodo impl, no el SHA del archivo adyacente; `check` ensucia 16 `.bilink` sólo con `resolved_at`; y `resolved_at` sólo se usa como baseline de `git log --since=` en tres lugares.

Sin verificar directamente: todo lo que cita línea de código, y el "23% de captures duplicados" que el ADR toma de `capture.md`.

## Salida

Lista de hallazgos ordenada por gravedad. Por cada uno: dónde, qué está mal, y qué evidencia lo confirma. Separá lo que es error de hecho de lo que es objeción de diseño. Si algo está bien y parecía sospechoso, decilo también.
