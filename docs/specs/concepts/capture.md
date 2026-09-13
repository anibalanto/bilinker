# El capture

Un capture es una ubicación: qué archivo y qué nodo del AST. Nada más. No sabe qué versión del fragmento alguien aprobó, ni dónde cayó la última vez que se lo resolvió.

Un bilink no describe un fragmento: referencia un capture ([bilink.md](bilink.md)). Varios bilinks pueden referenciar el mismo.

## El archivo

### El id es el hash de la ubicación

El nombre del archivo es el hash de lo único que contiene: cada campo seguido de un `\0`.

```
id = sha256( file "\0" query "\0" )[..32]
```

El terminador va después de cada campo y no entre campos, y eso importa: así el id no cambia cuando un campo desaparece del formato. Es lo que permitió sacar el `offset` sin re-acuñar los 316 captures que existían.

De ahí salen tres propiedades que no hay que sostener con reglas. Un capture es inmutable: cambiarle la ubicación le cambiaría el nombre, así que no se cambia, se acuña otro. La deduplicación es por construcción: dos referencias a la misma ubicación producen el mismo id y por lo tanto el mismo archivo, sin buscar un capture equivalente antes de crear uno. Y mover un fragmento es un cambio visible: `apply` repunta el `link` a un capture nuevo, y ese repunte es un cambio en el bilink que alguien tiene que aprobar ([accept.md](accept.md)).

### El hash del contenido no entra en el id

El id sale de la ubicación y sólo de la ubicación. Si entrara el hash del fragmento, cada edición del código produciría un capture nuevo y el vínculo se rompería en cada commit: la referencia dejaría de sobrevivir a los cambios, que es su razón de ser.

Por el mismo motivo tampoco entra `commit`: la procedencia de una decisión no es parte de dónde está un fragmento.

### Los captures viven en `.bilink/capture/<id>.yaml`, al lado de los bilinks

```
<layer-root>/
  .bilink/
    <uuid>.yaml              ← las relaciones
    capture/
      <id>.yaml              ← las ubicaciones, inmutables
    cache/
      state                  ← lo derivable · no versionado
    version                  ← la versión de formato
    .gitignore               ← qué de acá adentro no se versiona
    index/
      index                  ← lookup O(1) · no versionado
```

Un capture vive en la capa cuyo archivo referencia; su `file` es relativo a la raíz de esa capa.

La extensión es `.yaml` y nada más. El tipo lo dice la carpeta que lo contiene, así que repetirlo en el nombre sería redundante.

### Un capture tiene dos campos, `file` y `query`

```yaml
# .bilink/capture/<id>.yaml
file: docs/specs/concepts/check.md
query: |-
  (section (atx_heading (inline) @n0 (#eq? @n0 "La verificación"))
    (section (atx_heading (inline) @n1 (#eq? @n1 "Los estados de aceptación"))) @target)
```

| Campo | Descripción |
|---|---|
| `file` | Path relativo a la raíz de la capa. |
| `query` | Query tree-sitter con una o más capturas `@target`. Ausente = el archivo completo. |

Dos campos, y los dos entran en el id. No hay más: ni `range`, ni `state`, ni `resolved_at`. Todo eso se puede reconstruir resolviendo la query, así que vive en [la cache](cache.md) y no en git.

Que el archivo tenga exactamente los campos que lo nombran es lo que hace verificable el id: cualquiera puede recalcularlo leyendo el archivo.

## El fragmento

### El fragmento son los `@target`, en orden de archivo

Una query puede llevar más de una captura `@target`. El fragmento es la concatenación de sus rangos, en el orden en que aparecen en el archivo, no en el que la query los nombra, que es un detalle de cómo se escribió el patrón y no del documento.

Con un solo `@target` el fragmento es ese nodo y nada más: es el caso de siempre.

Con varios se pueden decir dos cosas que un nodo solo no puede:

| | |
|---|---|
| menos que un nodo | la firma de un método sin su cuerpo: se capturan `type`, `name` y `parameters`, y `body` queda afuera |
| más que un nodo | una ruta que sale de dos anotaciones —`@RequestMapping` en la clase, `@GetMapping` en el método—, que como texto completo no existe en ningún nodo |

Y sigue siendo estructural: son nodos, no rangos de bytes. Si el código se mueve, la query lo vuelve a encontrar, que es la propiedad por la que un capture existe.

Ninguna parte contiene a otra. El fragmento es la concatenación, así que una parte adentro de otra se contaría dos veces y el hash pasaría a depender de un solapamiento que nadie quiso. Cuando se señala una posición que resuelve a un nodo que contiene a otro señalado, no hay capture raro que escribir: hay una posición que resolvió más arriba de lo que se apuntó, y decirlo es más útil que capturar cualquier cosa.

Pero dos partes sí pueden compartir una línea, y es el caso normal: en `public Dto get(String t)` el tipo de retorno y los parámetros son dos partes de la misma línea, con el nombre del método en el medio y afuera. No se solapan —son rangos disjuntos— y el fragmento no las cuenta dos veces. Lo que eso obliga es a que mostrar un fragmento no sea imprimir sus partes una detrás de otra: concatenadas, esa línea sale repetida. Es problema de quien lo muestra y no del formato, y lo resuelve [`get`](get.md) marcando el hueco intra-línea. El fragmento sigue siendo la concatenación, y el `hash` no cambia.

### Una query, no una lista de queries

Un patrón único ancla los fragmentos entre sí: dice *"el `@RequestMapping` de la clase que contiene al método `getPermissions`"*. Una lista de queries independientes ancla cada una por su cuenta, y la primera sería *"el `@RequestMapping`"* a secas, que en un archivo con dos clases matchea la equivocada.

Y abriría la resolución parcial, que no existe: tree-sitter matchea el patrón entero o no matchea. Con una lista, dos de tres queries resueltas no serían `RESOLVED` ni `UNANCHORED` sino un fragmento a medias, y habría que inventarle un estado.

Menor pero cierto: el id es `sha256(file \0 query \0)`, y `query` sigue siendo un string.

### El separador es `\n`

Los rangos no son contiguos, así que hay que decidir qué va entre uno y el siguiente, y eso entra en el `hash`:

| | |
|---|---|
| nada | dos capturas pegadas producen un texto que no existe en ningún archivo |
| `\n` | elegido: legible, y estable frente a cuánto espacio haya en el medio |
| el texto intermedio | es el archivo tal cual, pero entonces el cuerpo entra por la ventana cuando dos capturas lo tienen en el medio |

Y no se vuelve a tocar. Cambiarlo movería de una vez el hash de todos los captures multi-fragmento, y todos pasarían a `ALTERED` sin que nadie tocara el código.

`hash_ast` sigue la misma regla: las s-expressions de los nodos, en el mismo orden, unidas por `\n`.

### No hay sub-rango

Un capture nombra un conjunto estructural de nodos, uno por `@target`, cada uno entero. No se puede referenciar un rango de bytes adentro de uno, y la selección con la que se lo crea sirve para encontrar los nodos, no para recortarlos.

Un rango adentro de un nodo se corre con cualquier edición encima suya dentro del mismo nodo: su granularidad es ilusoria, se rompe todo el tiempo y hay que repuntarlo a mano. Un ancla de nodo entero es estable, y sus falsas alarmas son honestas: *"esto cambió, fijate si tu spec sigue valiendo"*.

Lo que se pierde es atribución, no detección: dos fragmentos de spec que describen dos partes de la misma función pasan a compartir capture. `hash` dice que cambió y `hash_ast` si fue sólo espaciado; cuál parte cambió lo dice `bilinker get --diff`, que es trabajo de quien mira.

Si hace falta más precisión, la respuesta es una query que nombre algo más chico, o que nombre varios nodos y deje el resto afuera, no un recorte sobre una que nombra algo más grande. Por eso una fila de tabla markdown se ancla por el texto de su primera celda: es un nodo, y tiene con qué distinguirse.

### El rango excluye el espacio que rodea al nodo

Dónde empieza un nodo depende de qué hay alrededor. En YAML el mismo item de secuencia empieza en el `-` cuando es el último y en la indentación de su línea cuando lo sigue otro: agregar un item más abajo le cambiaba los bytes, y con ellos el hash, a un item que nadie tocó.

Eso contradice la propiedad central, así que el rango resuelto se recorta en los dos bordes. El fragmento es su contenido; el espacio que lo separa de sus vecinos es de los dos y no de él.

Va en el único lugar donde un nodo se convierte en rango, así que no hay forma de obtener uno sin recortar.

Con varios `@target` se recorta cada rango por separado, antes de concatenar. Recortar la concatenación dejaría los bordes internos a merced de dónde termina un nodo y empieza el otro, que es justo el contexto del que el recorte existe para independizar.

## La referencia y el ciclo de vida

### Referencia desde un bilink

```yaml
endpoint:
  0:
    link: capture 67ba7217e0334051becd4921b55a7872
  1:
    link: path >impl
```

El prefijo `capture ` identifica el tipo de endpoint ([bilink.md](bilink.md), "Tipos de endpoint").

El capture no guarda nada de la aceptación. Eso vive en el bloque `accepted` del bilink, por endpoint, porque dos bilinks pueden haber aceptado versiones distintas del mismo fragmento: si A aceptó la v1 y B la v2, y el código está en v2, `check` debe reportar drift para A y no para B. Con un solo valor compartido eso sería imposible de expresar.

### Un capture no cambia; los `link` cambian de capture

Cuando un fragmento se mueve —el archivo se renombra, el anchor cambia de nombre— la ubicación nueva es otro capture. `apply` lo acuña y repunta el `link` del bilink que está corrigiendo. El capture anterior queda intacto para los demás referentes.

No hay copy-on-write ni regla de fork por tipo de fix: `apply` nunca muta, acuña, y ningún referente se entera de la corrección de otro.

Y repuntar no es gratis: `apply` repunta el `link` pero no devuelve el endpoint a `OK`. La ubicación cambió, y aprobar una ubicación es una decisión humana como aprobar un contenido. El endpoint queda en `RELOCATED` hasta que alguien acepte ([accept.md](accept.md)).

### Quién escribe qué

| Comando | Escribe |
|---|---|
| `bilinker capture` | Un capture nuevo, si no existía. Devuelve su id. |
| `bilinker check` | Nada en el capture. Escribe `range` y `state` en [`cache/state`](cache.md). |
| `bilinker apply` | Acuña captures y repunta un `link`. Nunca modifica uno existente. |
| `bilinker accept` | Nada en el capture. Escribe `accepted` en el bilink. |

Ningún comando modifica un capture existente. La única operación sobre el conjunto es agregar, y `prune` sacar los que ya no referencia nadie.

### Ciclo de vida

Un capture no conoce a sus referentes. `bilinker remove` sobre un bilink no borra los captures que referenciaba: puede haber otros usándolos.

Un capture sin referentes es basura inofensiva: ocupa un archivo y nadie lo lee. `bilinker capture prune` los elimina.

`prune` es mark & sweep sobre dos clases de raíz, no una sola. Un capture está vivo si lo referencia algún `link` —la ubicación vigente de un endpoint— o algún `accepted.link` —la ubicación que alguien aprobó—. Barrer sólo por la primera borraría el capture que dice dónde estaba lo que se aceptó, y con él la capacidad de decidir si una ubicación cambió.

### `prune` también mira adentro del vecindario

Un capture está referenciado si algún bilink lo nombra, y con el vecindario siendo captures `n.1.link` es una forma de nombrarlo, tanto en la declaración como en cada entrada de `accepted`.

Sin esa regla el primer `prune` se lleva los vecinos de todos los endpoints con cierre de firma, y lo que queda es un `accepted` apuntando a captures que ya no existen: un `UNRESOLVED` masivo producido por una limpieza.

No es un caso especial, es la misma regla: se borra lo que nadie referencia, y `n.1.link` referencia.

### El fan-out vive del lado del capture

Un bilink tiene siempre exactamente dos endpoints. La multiplicidad la aporta el capture: un fragmento puede tener N bilinks asociados.

```
                    ┌── bilink → spec de validación
capture(vote) ──────┼── bilink → ADR de auditoría
                    └── bilink → issue 3a
```

Esto cierra la alternativa de darle aridad variable al bilink. Un archivo llamado bilink con `link.0` … `link.4` sería una contradicción, y la aridad variable obligaría a redefinir la topología de cadena —lineal, con exactamente dos tips— y el copiado de valores aceptados de los endpoints `path`, que asume un único endpoint estructural adyacente.

Lo que esta forma no expresa es una relación conjunta entre tres cosas. Una estrella dice *"D se relaciona con A"* y *"D se relaciona con B"* por separado; no dice *"D gobierna el vínculo entre A y B"*. Para eso hace falta que un bilink apunte a otro bilink: el endpoint de tipo bilink, que está especificado y no implementado en la decisión `bilink-endpoint`.

### Relación con las cadenas

Las cadenas no cambian por el capture. Un bilink sigue viviendo en su capa, con endpoints `path` hacia las capas vecinas y la misma propagación por copia. Lo único que el capture agrega es que sus endpoints estructurales referencian captures locales.

Un bilink nunca referencia un capture de otra capa: eso rompería la propiedad de que aceptar en una capa nunca escribe en el repo de otra, que es lo que evita la cascada circular. Las conexiones entre capas siguen siendo endpoints `path`.

La excepción aparente es `accepted.link` de un endpoint `path` o `repo`, que copia el id del capture ajeno. Es una copia opaca: se compara, no se resuelve. Nadie va a buscar ese archivo en la capa local.

### Relación con lattice

Un capture es, casi literalmente, un nodo del grafo de lattice: su forma canónica es `<layer-root>::<file>#<range>`. Un bilink es una arista sobre captures.

El `range` sale de la cache, no del capture. Un clon fresco no lo tiene hasta que corra un `check`.

## El comando `capture`

### La selección elige un nodo, no un rango

```
bilinker capture <file> [<start_line>:<start_col> <end_line>:<end_col>] [--dry-run]
```

| Argumento | Tipo | Descripción |
|---|---|---|
| `file` | path | Ruta del archivo, relativa a la raíz de la capa actual. |
| `start_line:start_col` | int:int | Línea y columna de inicio de la selección (1-based). Omitir para capturar el archivo entero. |
| `end_line:end_col` | int:int | Línea y columna de fin de la selección (1-based). |
| `--dry-run` | flag | Imprime el capture que se crearía sin escribir nada. |

La selección se usa para encontrar el ancla estable que la contiene, y después se descarta: el capture es ese nodo entero. Seleccionar media función captura la función.

```bash
$ bilinker capture architecture.md 34:10 34:52
3a4b5c6d2e3f4a5b9c6d7e8f9a0b1c2d
```

```yaml
file: architecture.md
query: |-
  (section
    (atx_heading (inline) @n0 (#eq? @n0 "Decisión"))
    (paragraph) @target)
```

La selección cubría media línea del párrafo; el capture es el párrafo. Para capturar algo más chico hace falta una query que lo nombre, no un recorte sobre una que nombra algo más grande.

### Sin selección, el capture es el archivo entero

Sale sin `query`, que es lo que "Un capture tiene dos campos, `file` y `query`" define como el archivo completo. No hay nodo AST que encontrar, así que tampoco hay ancla que verificar ni lenguaje que soportar: un archivo entero se captura aunque no haya gramática para él.

### Lenguajes soportados

| Extensión | Lenguaje | Anclas estables |
|-----------|----------|-----------------|
| `.java` | Java | `class_declaration`, `interface_declaration`, `enum_declaration`, `method_declaration`, `constructor_declaration`, `field_declaration` |
| `.rs` | Rust | `function_item`, `struct_item`, `enum_item`, `trait_item`, `impl_item`, `const_item`, `static_item` |
| `.yaml`, `.yml` | YAML | `block_sequence_item` (usa `id:` como predicado), `block_mapping_pair` (usa clave) |
| `.md` | Markdown | `section` (usa texto del heading como predicado), `pipe_table_row` (usa el texto de su primera celda) |
| `.ts`, `.js` | TypeScript | `class_declaration`, `abstract_class_declaration`, `function_declaration`, `generator_function_declaration`, `enum_declaration`, `interface_declaration`, `type_alias_declaration`, `method_definition`, `method_signature` |
| `.tsx`, `.jsx` | TSX | igual que TypeScript, con parser TSX para archivos con JSX |

El lenguaje se determina por la extensión del archivo. Una extensión sin gramática se trata como texto plano, y ahí no hay `hash_ast` ni `RESTYLED`.

### Ánclas estables recomendadas

Qué nodo conviene capturar, por tipo de documento:

| Tipo de documento | Ánclas estables | Frágil (evitar) |
|---|---|---|
| Código (Java, Rust, TypeScript…) | función, método, clase, declaración con nombre | comentario, `use`/`import` |
| Markdown | heading h2–h4, bloque de código, fila de tabla | párrafo libre, h1 |
| YAML / TOML | clave de mapping, item con `id:` | valor string libre |
| JSON | clave de objeto | valor primitivo |

El criterio es que el ancla se nombre a sí misma. Un nodo sin nombre propio produce una query que matchea el primero de su tipo en el archivo, y un capture así apunta a otra cosa sin fallar. `bilinker capture` lo verifica antes de escribir y falla si no puede identificar el fragmento unívocamente.

### `capture` sube en el AST hasta el primer ancestro estable y verifica que la query identifique el fragmento

1. Leer el archivo y parsearlo con la gramática tree-sitter del lenguaje detectado por extensión.
2. Encontrar el nodo AST más pequeño que contiene la selección completa (`named_descendant_for_point_range`). Con varias selecciones, una vez por cada una.
3. Subir en el árbol AST hasta el primer ancestro que sea un ancla estable para el lenguaje.
4. Casos especiales por lenguaje. YAML `block_sequence_item`: busca el par `id:` dentro del item y usa su valor como predicado, capturando el item completo. YAML `block_mapping_pair`: usa el texto de la clave como predicado. Markdown `section`: busca el heading dentro del section y usa su texto inline como predicado, capturando toda la sección (heading + contenido). Rust `impl_item`: el discriminante no es un campo `name` sino el tipo implementado (`type:`) y, cuando es la implementación de un trait, también el trait (`trait:`). Con uno solo, `impl Foo` y `impl Bar for Foo` quedan indistinguibles.
5. Construir la query como el camino del AST desde ese ancestro hasta el nodo target. Cada predicado usa un nombre de captura único (`@n0`, `@n1`, …). El `@target` se coloca en el nodo que representa el fragmento completo. Con varios nodos señalados —[`chain new`](chain.md) con más de una posición— el ancestro es el ancla estable que los contiene a todos, los caminos se funden en un patrón único, y va un `@target` por nodo. Las partes de un patrón salen en orden de archivo: tree-sitter exige el orden de la gramática, y en Java las anotaciones van antes del nombre. Ninguna parte puede contener a otra: cuando pasa, `capture` falla diciendo cuáles.
6. Verificar que la query identifica el fragmento: resolverla contra el mismo archivo y comprobar que devuelve exactamente un match, con exactamente los nodos señalados. Si devuelve otros nodos, un número distinto, o matchea más de una vez, `capture` falla sin escribir nada.
7. Calcular el id —el hash de los campos, cada uno seguido de un `\0`— y escribir `.bilink/capture/<id>.yaml` si no existe. Nada de cache: ni `range`, ni `state`, ni timestamp.

`capture` no calcula ni almacena hashes: un capture describe ubicación, no contenido aceptado. El hash lo establece `bilinker accept` en el bilink que lo referencie.

### El nombre entra escapado

Un predicado es un string adentro de una query, así que `\` y `"` no se pueden escribir tal cual: `\n` adentro de una query es un salto de línea y no dos caracteres, y un `"` cierra el string y hace inválida la query entera. Se escapa en un solo lugar, el mismo que usa `recapture` al reescribir el nombre: escribir y reescribir el mismo campo con reglas distintas hace que el mismo nombre produzca dos queries según por qué camino se generó.

Ningún nombre queda afuera por cómo se escribe: un ancla con `\` o con `"` en el nombre se captura igual. Lo que el predicado guarda es el nombre, no una versión suya que se pueda escribir sin escapar.

### stdout lleva el id del capture; stderr, la metadata

stdout lleva el id del capture, para referenciar desde un `link`:

```
67ba7217e0334051becd4921b55a7872
```

stderr lleva metadata informativa:

```
created: .bilink/capture/67ba7217e0334051becd4921b55a7872.yaml
file:    src/main/java/ar/example/demo/persona/Persona.java
anchor:  class_declaration "Persona" → method_declaration "vote"
```

Si el capture ya existía, `stderr` dice `reused:` en vez de `created:` y no se escribe nada. Es el caso normal de capturar dos veces la misma ubicación, y no es una condición especial: el id sale del contenido, así que el mismo fragmento produce el mismo archivo.

El id va solo a stdout para poder usarlo en pipes:

```bash
id=$(bilinker capture src/lib.rs 10:1 24:2)
```

### Código de salida de `capture`

| Código | Condición |
|---|---|
| 0 | Capture creado (o `prune` completado). |
| 1 | Error: archivo no existe, selección fuera de rango, lenguaje sin gramática. |

### Propiedades garantizadas de `capture`

- Unicidad de la referencia: la `query` resuelve a los nodos que se señalaron, y a ninguno otro. Un ancla sin discriminante —un `impl` sin tipo, un comentario, un `use`— produce una query que matchea el primer nodo de ese tipo del archivo: un capture que apunta a otra cosa y no falla. `capture` verifica antes de escribir y falla si no puede identificar el fragmento unívocamente. Un capture mal anclado es peor que uno roto, porque reporta OK sobre una correspondencia que no existe.
- Determinismo de la referencia: dos ejecuciones sobre el mismo archivo y selección sin modificaciones intermedias producen la misma `query` y los mismos rangos. El orden en que se pasan las posiciones no cambia nada: las partes van en orden de archivo.
- Reuso: capturar dos veces el mismo fragmento devuelve el mismo id, sin buscar nada.
- Independencia de git: `capture` no requiere que el archivo esté bajo control de versiones. Sí lo requiere `accept`, que necesita el commit del contenido aprobado.
- No toca bilinks: `capture` crea el archivo del capture y nada más. Referenciarlo desde un `link` es un paso aparte, vía `bilinker chain new` o `recapture`.

## `capture prune` y `capture remove`

### `bilinker capture prune`

Elimina los captures de la capa que no alcanza ningún bilink.

```
$ bilinker capture prune

3 captures sin referentes:
  9f8e7d6c…  crates/bilinker/src/sciplink.rs
  1a2b3c4d…  crates/bilinker/src/scip_index.rs
  5e6f7a8b…  crates/bilinker-cli/src/main.rs :: (function_item "scip_retrofit")

Eliminar? [y/N]
```

Un capture huérfano no rompe nada: nadie lo lee. `prune` es higiene, no reparación.

Como los captures son inmutables, cada vez que `apply` corrige una ubicación acuña uno nuevo y el viejo queda: vivo mientras algún `accepted.link` lo nombre, huérfano cuando esa aceptación se reemplace.

### `bilinker capture remove`

```
bilinker capture remove <uuid> [--force]
```

Elimina un capture puntual. Acepta un prefijo del id, y falla si es ambiguo.

Se niega si tiene referentes, porque borrarlo dejaría bilinks apuntando a la nada:

```
$ bilinker capture remove 5fdff600
Error: el capture 5fdff600 tiene referentes — usar `bilinker recapture` para repuntarlos, o --force
```

`--force` lo borra igual y avisa que hay que correr `check`. `prune` es lo que corresponde para limpiar en bloque; esto es para un capture concreto.

## El comando `recapture`

### Repunta un endpoint estructural a otro fragmento

```
bilinker recapture <uuid>.<N> <file> [<pos>] [<end>]
```

| Argumento | Descripción |
|---|---|
| `<uuid>.<N>` | Endpoint a repuntar. Acepta prefijo del UUID. |
| `file` | Archivo del fragmento nuevo, relativo a la raíz de la capa. |
| `pos` | Posición `línea:col` (1-based). Omitir para capturar el archivo completo. |
| `end` | Fin de la selección `línea:col`. Default: igual que `pos`. |

Existe porque hay estados que no tienen auto-fix y tampoco se resuelven aceptando. Una sección de spec renombrada o un test reescrito dejan el capture en `UNANCHORED`: el fragmento ya no está donde estaba, y `apply` no puede adivinar dónde quedó. Sin este comando la única salida es editar el `link` a mano: un reemplazo de texto sobre el campo que define a qué apunta un vínculo, sin validar que el capture exista, que esté en la misma capa, ni que el endpoint sea estructural.

1. Resolver el bilink y verificar que el endpoint sea estructural. Un endpoint `path` o `issue` no tiene capture que repuntar.
2. Crear el capture del fragmento nuevo; reusa uno existente si la referencia `(file, query)` es idéntica, igual que `bilinker capture`.
3. Escribir el `link` apuntando al capture nuevo.
4. Limpiar `state.N`: el estado anterior describía el capture viejo, y dejarlo mentiría hasta el próximo `check`.
5. Reportar si el capture anterior quedó sin referentes.

```
$ bilinker recapture 430a5d51.0 docs/specs/concepts/check.md 69:9

5fdff600-8690-4a75-85f6-e91072489d67
link.0 → capture 5fdff600
  antes: fbd0ca92  (quedó sin referentes)

revisar con `bilinker get 430a5d51.0` y aceptar con `bilinker accept 430a5d51.0`
```

El id del capture va a stdout para poder usarlo en pipes; el resto a stderr.

### `--as` regenera la query con un generador, y el endpoint lo anota

```
bilinker recapture <uuid>.<N> <file> <pos> --as <modo>
```

La posición se resuelve igual que en [`chain new --as`](chain.md): el generador escribe la query de lo que se señaló, y el endpoint anota en `as` con qué se capturó. Un modo que no existe es un error que lista los que hay, y no repunta nada.

Es cómo un capture escrito con otra regla pasa a la de hoy sin crear otro bilink. Un bilink nuevo tendría otro UUID, y un endpoint `abstract` es exactamente un UUID del que otro repo está colgado.

### `recapture` no acepta

Corrige a dónde apunta el endpoint y nada más. Que el contenido del fragmento nuevo sea el correcto lo decide un humano con `accept`.

Es la misma separación que entre `apply` y `accept`, y por el mismo motivo: reparar una ubicación es mecánico y verificable; afirmar que un contenido sigue cumpliendo lo que el vínculo promete, no.

### `recapture` no borra el capture anterior

Puede tener otros referentes: la deduplicación hace que un capture sea compartido con frecuencia. Se informa que quedó huérfano y se limpia con `bilinker capture prune`.

Borrar por si acaso dejaría bilinks apuntando a la nada; dejar un archivo huérfano no rompe nada.

### Cuándo usar `recapture`

- El capture está en `UNANCHORED`: la query no matchea y la similitud no alcanzó para `REANCHORED`.
- El fragmento se movió a otro archivo, y no fue un rename que git pueda detectar: el caso de código que migra a otro repo.
- El vínculo sigue teniendo sentido pero conviene apuntarlo a un fragmento distinto.

Para lo que sí tiene auto-fix —`MOVED` y `REANCHORED`— corresponde [`apply`](apply.md), que recalcula el fix en vez de pedir la posición.

| Código | Condición |
|---|---|
| 0 | Endpoint repuntado. |
| 1 | UUID no encontrado, endpoint no estructural, el `link` ya apuntaba a ese capture, o el fragmento nuevo no se pudo capturar. |

## Invariantes

1. El nombre de un capture es el hash de sus campos, cada uno seguido de un `\0`, y el archivo contiene exactamente esos campos.
2. Un capture es inmutable. Ningún comando modifica uno existente.
3. Un capture describe ubicación, nunca aceptación. No contiene hashes ni commits.
4. `file` es relativo a la raíz de la capa donde vive el capture.
5. Un capture nombra un conjunto de nodos enteros, uno por `@target`: no hay sub-rango. El fragmento es la concatenación de sus rangos en orden de archivo, separados por `\n`. Los rangos absolutos son derivados y viven en la cache.
6. Ninguna parte de un fragmento contiene a otra: los rangos son disjuntos.
7. Un `link` sólo referencia captures de su propia capa. Un `accepted.link` de endpoint `path` o `repo` puede contener una copia opaca de un id ajeno, que no se resuelve localmente.
8. Un capture puede ser referenciado por cualquier cantidad de bilinks, incluido cero.
9. `apply` acuña captures y repunta un `link`; nunca escribe `accepted`.
10. `accept` escribe `accepted`; nunca toca un capture.
11. Borrar un bilink nunca borra un capture.
12. `prune` conserva todo capture alcanzable desde un `link` o un `accepted.link`, incluidos los de `n.1.link`.
