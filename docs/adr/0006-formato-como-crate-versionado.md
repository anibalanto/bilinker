# ADR-0006: bilinker — El formato como crate versionado

**Estado:** Propuesto **Fecha:** 2026-08-29

**Parte de** [la épica del MVP](../../../../../../.stratum/worklist/1.epic.md). Depende de [ADR-0003](0003-formato-captures-y-aceptacion.md), que decide **qué** es el formato; éste decide **dónde vive** y **cómo se versiona**.

---

## Contexto

ADR-0003 introduce `.bilink/version` y apoya en él dos cosas: que un binario no lea archivos que no entiende, y que un consumidor de la frontera pueda negarse en vez de malinterpretar. Pero deja el campo **declarado a mano**, y eso tiene un modo de falla obvio: alguien cambia el formato y no lo sube.

No es hipotético. [ADR-0005](0005-frontera-entre-proyectos.md) agrega los endpoints `repo` y `abstract` **sin migración**, porque son aditivos — y sin embargo un parser viejo leería `abstract` como un path de capa y no fallaría. Un cambio así es exactamente el que se olvida de bumpear.

Y hay un segundo hecho: el parser del formato hoy no existe como unidad. Está repartido entre `bilink.rs` (380 líneas), `link.rs` (354) y la parte de `capture.rs` que mezcla el formato con el algoritmo de captura por tree-sitter.

---

## Decisión

### 1. El formato es un crate, y su versión es la del formato

Se extrae a un crate propio lo que define el formato: los tipos con `serde` y `schemars` de ADR-0003, su serialización, y el esquema JSON generado. Todo lo demás —`check`, `accept`, `apply`, la resolución de queries— depende de él.

**La versión del crate es la versión del formato.** No se puede cambiar el parseo sin releasear, ni releasear sin bumpear, así que el campo `.bilink/version` deja de ser una promesa y pasa a ser una propiedad del artefacto.

**Y se verifica, no se declara.** Un test afirma que el hash del esquema generado corresponde a la versión registrada:

```
sha256(esquema generado)  ==  <hash registrado para la versión N>
```

Cambiar los tipos sin subir la versión **falla el test**. La versión ordinal sirve para comparar y para leer; el hash garantiza que corresponde a lo que dice ser. Es el mismo principio que el ADR aplica a los captures: direccionar por contenido para que la identidad no dependa de que alguien se acuerde.

### 2. Las migraciones viven al lado, y pinean dos versiones

Una migración existe para llevar archivos de lo que entiende el formato N a lo que entiende el N+1. Son dos mitades de la misma cosa, así que van en la misma carpeta que el formato.

Cada migración **declara qué par de versiones puentea y depende de las dos**. Ésa es la única forma de que un componente pueda leer los dos formatos, y es lo que hace que la verificación de que la migración no perdió nada la haga **la migración** — no `check --against`, que linkea un solo parser.

**El crate del formato tiene sólo el formato vigente.** Cargar todos los parsers históricos en el camino de lectura es lo que `concepts/migration.md` descarta explícitamente: *"eso funciona hasta el segundo cambio, y a partir de ahí cada lectura carga con toda la historia del formato"*.

Pero conviene decir el costo, porque es real y distinto: **la historia sí se acumula en el build.** Si `002` depende de los crates de formato 1 y 2, esos crates quedan en el repo para siempre. Lo que no se acumula es lo que cada lectura carga.

Y de ahí sale una regla que `migration.md` todavía no dice: **el conjunto de migraciones es de sólo-agregar.** Nunca se borra una, ni siquiera cuando parece que ya nadie está en ese formato — es lo único que permite que alguien parado en una versión vieja llegue a la actual corriendo la cadena entera.

### 3. El esquema se publica, y es lo que la frontera consume

`schemars` genera el esquema JSON a partir de los tipos. Se publica como artefacto de la release.

Ahí rinde de verdad, y es la razón más fuerte de todo el ADR: hoy [ADR-0005](0005-frontera-entre-proyectos.md) hace que `retinar` lea los `.bilink` de `hsi` **confiando** en entenderlos. Con el esquema publicado, valida antes de interpretar — y lo hace **sin adoptar bilinker**, con cualquier validador de JSON Schema en cualquier lenguaje.

Eso baja el costo de adopción del lado del proveedor, que es el riesgo que la épica viene tratando de minimizar. Es el mismo movimiento que un `api-model`: publicar la superficie para que otro la consuma sin depender de tu build.

**La dirección es Rust → esquema, no al revés.** Los tipos con serde son la fuente y el esquema sale generado. La alternativa —esquema a mano, tipos generados con `typify`— tiene un argumento fuerte, que el formato deje de estar definido en Rust; pero cuesta codegen en el build y hoy no hay otro proyecto implementando su propio lector. Si aparece, es el momento de darlo vuelta.

---

## Consecuencias

**Invariantes nuevas.** La versión de formato es la del crate y se verifica contra el hash del esquema generado · una migración depende de los dos formatos que puentea · el conjunto de migraciones es de sólo-agregar · el crate del formato contiene sólo el formato vigente.

**Lo que cuesta.** Hay que partir `capture.rs`, que hoy mezcla el formato del `.capture` con el algoritmo de captura por tree-sitter — no es mover archivos. Y el árbol de crates crece con cada versión de formato, aunque el binario del día a día linkee sólo el último.

**Lo que se va.** Las ~730 líneas de parser artesanal entre `bilink.rs` y `link.rs`, reemplazadas por `serde` más el destructurado del prefijo — que con el tipo explícito de ADR-0003 es partir en el primer espacio y matchear seis palabras.

**Trabajo que dispara en las specs.** `concepts/migration.md`: la distinción entre *migración* y *proceso de migración*, y la regla de sólo-agregar. Y un lugar donde documentar el esquema publicado y su política de versiones, que hoy no existe.

---

## Alternativas descartadas

- **Declarar la versión a mano en `.bilink/version`.** Es lo que ADR-0003 dejó planteado, y no cubre el modo de falla que importa: un cambio aditivo que nadie bumpea. El hash del esquema lo cierra sin pedirle disciplina a nadie.
- **Derivar la versión de formato del ledger de migraciones.** El ledger es por repo y la versión es por carpeta, así que no responde durante la coexistencia, cuando conviven dos formatos a propósito. Y no puede expresar un cambio aditivo, que no deja entrada en el ledger.
- **Un crate que acumule todos los parsers históricos.** Es lo que `migration.md` descarta en su § "Por qué no alcanza con que la herramienta lo tolere": funciona hasta el segundo cambio.
- **Esquema a mano con `typify` generando los tipos.** Saca la definición del formato de Rust, que es su virtud, pero agrega codegen al build para un beneficio que hoy nadie cobra — ningún otro proyecto implementa un lector. Se revisa cuando alguno lo haga.
- **Llamar al crate por la migración** (`bilink-migrator` o similar). El migrador es un consumidor del formato como cualquier otro; `check`, `accept` y `apply` dependen del parser tanto como él. El artefacto es el formato.
