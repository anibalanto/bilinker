# La migración del formato

Cómo los metadatos de una capa pasan de un formato al vigente: el comando `migrate`, las migraciones que existen, el problema de bootstrap, lo que no necesita migración, y `restore-n1`, que devuelve lo que una migración descartó y por eso no es una migración. El mecanismo general —una migración es una función pura de sus archivos de entrada, un ledger por repo, un id por migración— es de accreta; acá está lo que bilinker hace con él.

## `bilinker migrate`

### Migra los metadatos de una capa al formato vigente

```
bilinker migrate [<path>] [--recursive] [--dry-run]
```

| Argumento | Descripción |
|---|---|
| `path` | Capa a migrar. Default: la capa actual. |
| `--recursive` | Migra también todas las capas descendientes encontradas en `.stratum/`. |
| `--dry-run` | Muestra qué haría sin escribir nada. |

Es idempotente: correrlo dos veces no hace nada la segunda. `--dry-run` no escribe ni captures, ni bilinks, ni ledger. No commitea: se revisa con `git diff` y se commitea a mano. Y no resuelve: no corre tree-sitter ni git, así que después se corre `bilinker check .`.

### Correr `--recursive` no es opcional cuando un repo tiene varias capas

El ledger es por repo y las migraciones corren por capa. Si un repo contiene varias capas —el repo de specs de un proyecto suele tener la suya y las de sus subproyectos— hay que alcanzarlas todas en la misma corrida. Invocar `migrate` capa por capa registra la migración al terminar la primera, y las siguientes la ven registrada y se saltean, quedando sin migrar.

### Una migración retirada no se borra del ledger; se borra del registro

| Id | Estado | Qué hace |
|---|---|---|
| `bilinker-001-capture-split` | retirada | Extrajo la ubicación de cada endpoint estructural a un capture y reemplazó `link.N` por `capture <id>`. |
| `bilinker-002-file-partition` | vigente | Reescribe cada bilink a YAML, con los endpoints bajo `endpoint.0`/`endpoint.1` y el tipo de cada `link` explícito. Lo derivable sale a la cache. |
| `bilinker-003-accepted-list` | vigente | `accepted` pasa de objeto a lista, y el vecindario adquirido pasa a `declined`. |

`001` corrió en todos los repos que existían y su id sigue en cada ledger —quitarlo sería reescribir lo que pasó—, pero su código ya no está: `002` lee la forma embebida directamente, así que un repo que nunca corrió `001` se migra igual, en un paso. Es la asimetría que hace útil al ledger: registra qué le pasó a este repo, no qué sabe hacer este binario.

## Las migraciones

### `bilinker-001-capture-split` (retirada)

Convertía los endpoints con la ubicación embebida —`file :: query :: offset`— al formato con capture aparte, deduplicando: dos endpoints con `(file, query, offset)` idénticos compartían un capture, porque referencias idénticas describen la misma ubicación. Descartaba también `subgraph.N`, campo eliminado del formato.

Su trabajo lo hace `002` en una sola pasada. El id de un capture sale de la ubicación, así que la dedup es por construcción y no hace falta un paso que la produzca; y `002` lee la forma embebida además de la de `001`, así que no hay orden que respetar entre las dos. Un repo que nunca corrió `001` llega igual al formato 2.

### `bilinker-002-file-partition`

De `clave: valor` plano a YAML. `hash.N` pasa a `accepted.hash` y `hash_ast.N` a `accepted.hash_ast`; `commit.N`, `state.N`, y el `range` y el `state` que salen del capture van a la cache. Se escribe `.bilink/version`.

`resolved_at` se descarta en los dos archivos: no se muda a la cache, desaparece del formato.

`kind` y `name.N` se preservan, y `name.N` pasa a ser `name` adentro de su endpoint. Que la frase sea cierta costó una versión del lector de formato 1: no los modelaba, así que la migración recibía nada y esta línea describía algo que no pasaba. Está en la spec de la versión del formato, "Un formato que ya no se escribe todavía se lee".

`accepted.link` se siembra copiando `link.N` donde había `hash.N`. Es exacto donde el endpoint estaba `OK`: en el formato viejo un endpoint `OK` es uno cuyo contenido actual coincide con el aceptado en la ubicación que `link.N` describe, así que esa ubicación es la bendecida. Donde estaba no-OK es la única lectura disponible —el formato viejo no distingue drift de ubicación de drift de contenido— y es la que preserva la invariante de aceptación sin poner todos los bilinks en `RELOCATED` de golpe ni degradarlos a `PENDING`, que borraría el inventario de trabajo. En un endpoint `PENDING`, `accepted` queda ausente y sólo sobrevive `link`.

### Los captures se acuñan en la misma pasada

Cada capture se acuña bajo el hash de sus campos y se repuntan las dos clases de referencia: `link` y `accepted.link`.

El sub-rango se descarta, y se cuenta. El formato 3 no lo tiene: un fragmento es un nodo entero. Reubicarlo exigiría resolver la query y buscar el nodo correcto, y una migración no corre tree-sitter, así que el endpoint queda apuntando al nodo que lo contenía y el resumen dice cuántos, la misma regla que `001` con `subgraph.N`.

No tiene fan-out. Como el id no depende del hash del contenido, dos bilinks que aceptaron contenidos distintos del mismo fragmento siguen compartiendo capture, y la divergencia queda en sus `accepted`. Dos captures con la misma ubicación colapsan en uno: es la dedup por construcción, aplicada de una vez a lo que ya existía. Separarlo en una migración propia habría creado un formato intermedio —captures en YAML con su uuid viejo— que exige un crate propio para siempre y en el que nadie iba a estar.

### `bilinker-003-accepted-list`: `accepted` pasa a lista y el vecindario a `declined`

Dos cambios de forma, de 3.8 a 4.0, y uno de los dos no se puede completar desde una migración.

`accepted` pasa de objeto a lista, y eso es mecánico: un objeto se vuelve una lista de uno y no se pierde nada. Un endpoint con una decisión sigue teniendo una.

`n` gana `link` —un capture por vecino— y ahí no hay nada que traer. Los `n` ya escritos salieron de hashear ubicaciones crudas; convertirlos en captures exige resolver los tipos de la firma, y eso necesita un language server. Una migración es una función pura de los archivos de entrada: no consulta git, no resuelve queries tree-sitter, no lee la hora. Así que no puede, y no debería.

### El vecindario pasa a `declined` porque una migración no puede resolver tipos

De las salidas que se consideraron, dos son peores:

| | |
|---|---|
| descartar el `n` | baja la cobertura en silencio, y la ausencia sin marca significa otra cosa: que el fragmento no tiene firma resoluble |
| negarse si hay algún `n` adquirido | deja la migración bloqueada por algo que no puede arreglar, y obliga a re-aceptar todo antes de poder leer los archivos con el binario nuevo |
| escribir `declined` | la renuncia queda escrita |

Con `declined`, `check` y `status` dicen que ese endpoint no vigila su vecindario en vez de dejar creer que sí; un `accept` posterior con proveedor lo levanta solo, porque una renuncia anterior se levanta sola en cuanto hay con qué resolver; y nadie tiene que volver a tipear `--no-n1` en el medio. Al revés funciona y de frente no: se migra, y quien quiera el vecindario lo recupera aceptando.

### Una migración que no puede llevar un campo hacia adelante escribe `unknown`, no una renuncia

Las tres salidas de arriba eran tres de cuatro. Faltaba conservar los hashes y declarar que falta la ubicación —el `link: unknown` de un nivel—, que entrega lo que `declined` buscaba sin pagar lo que costó: la renuncia se ve escrita igual, y los dos sha256 por endpoint no se van. La migración tuvo los hashes en la mano: los lee del archivo de entrada, y lo único que no podía derivar era el `link`. Lo que le faltó fue un formato de salida capaz de escribir un nivel sin ubicación.

`declined` es una respuesta —nadie vigila el vecindario de este endpoint— puesta donde iba una imposibilidad —no pude traer los captures—. Y una renuncia escrita se lee de vuelta, así que los vecindarios degradados seguirían renunciando para siempre sin que nadie volviera a decidirlo. Es la regla que rige de acá en adelante: donde el tipo de salida no modela el hueco, la pérdida está en la firma, y la migración registra el hueco en vez de decidir por alguien. Devolver lo que `003` descartó no es otra migración: es `restore-n1`.

### El conteo va en las notas, y no es cosmético

Cuántos vecindarios se degradaron se reporta:

```
113 aceptación(es) envueltas en lista; 113 vecindario(s) pasaron a `declined` —
sus captures no se pueden derivar sin un language server. Recuperarlos es aceptar
con `lspd` vivo.
```

Una renuncia masiva escrita sin decirlo sería indistinguible de 113 personas que decidieron renunciar, y ésa es exactamente la confusión que el campo existe para no tener.

### Lee la forma vieja con tipos locales, y no con un crate congelado

El formato 1 pedía un crate propio —`bilink-format-v1`— porque era otra serialización entera. 3.8 y 4.0 son el mismo YAML y difieren en dos lugares: un crate para eso sería más código que el puente.

Y lo que no cambia no se enumera: se lee crudo y se copia. Listar los campos que quedan igual es lo que hace que una migración se rompa con el próximo campo aditivo.

## El problema de bootstrap

La herramienta que cambia de formato es la que se usa para cambiarlo, y las specs que describen el formato están bilinkeadas al código que lo implementa. Durante la transición conviven specs viejas y nuevas, binario viejo y nuevo, y bilinks en los dos formatos.

### Coexistencia por path: la migración escribe en `.bilink-migrate-<id>/` y deja `.bilink/` intacto

Los dos formatos no pueden ocupar `.bilink/` a la vez, así que la migración escribe en un path transitorio:

```
.bilink/  →  .bilink-migrate-002-file-partition/
```

El binario viejo sigue trabajando contra `.bilink/` sin enterarse; el nuevo se ejerce contra el path nuevo con datos reales antes de que nada sea irreversible. Los dos corriendo en el mismo instante sobre el mismo repo, cada uno contra la carpeta que entiende. `.git/info/exclude` recibe `.bilink-migrate-*` al empezar: esas carpetas son temporales y nunca se commitean.

### El path lleva el id de la migración

Con una sola el nombre parece de más, y no lo es: distingue una carpeta en curso de una abandonada de un intento anterior, y deja la puerta abierta a encadenar si alguna vez hay dos. El prefijo `bilinker-` se omite: dentro del directorio de bilinker es redundante.

### Es un derivado, no un espacio de trabajo

No se edita a mano: si se lo edita, deja de poder regenerarse, que es lo único que lo vuelve seguro. Y tiene que poder regenerarse, porque si entre la generación y el corte alguien acepta algo con el binario viejo, la copia migrada queda vieja y el corte se comería esa aceptación.

### Antes del corte `migrate` siempre regenera; después del corte el ledger la vuelve no-op

La idempotencia tiene dos regímenes. Antes del corte, `migrate` siempre regenera, y la regla operativa es regenerar justo antes de cortar. Después del corte el ledger la vuelve no-op, que es lo que el mecanismo general de migración exige.

### La entrada en el ledger va en el corte, no en la generación

Si se escribiera al generar, el repo quedaría marcado como migrado mientras sigue corriendo el formato viejo. Se registra cuando el estado es verdadero, no cuando el trabajo empezó.

### El corte deja el formato anterior en `.bilink-formato-<N>/`

El corte escribe el formato vigente en `.bilink/` y deja el anterior en `.bilink-formato-<N>/` al lado, y lo dice al terminar. Ese directorio no está en git: lo tapa el glob `.bilink-formato-*` del exclude, así que un clon fresco no lo trae y un árbol limpiado lo pierde. Dónde vive ese backup y cuándo se borra es una decisión abierta del impl.

## Lo que no necesita migración

### Un cambio aditivo no lleva migración, y por eso `.bilink/version` hace falta además del ledger

Los endpoints `repo` y `abstract` de la frontera son aditivos: ningún archivo existente los usa y todos siguen siendo válidos. La frontera se adopta bilink por bilink.

Que un cambio aditivo no lleve migración es justamente por qué `.bilink/version` hace falta además del ledger: un parser viejo leería `abstract` como un path y no fallaría, y el ledger no puede expresar eso porque no hubo migración que registrar.

### Mover los bilinks a una ref no es una migración

No transforma ningún archivo —los deja idénticos y cambia dónde viven— y una migración no puede consultar git, que es todo lo que esa operación hace. Es el corte a la ref, que hace `track`.

## Salida y código de salida

### El reporte se agrupa por repo, porque cada uno tiene su propio ledger

```
$ bilinker migrate --recursive --dry-run

repo /home/anibal/Workspace/accreta
  bilinker-001-capture-split  [dry-run]
    /home/anibal/Workspace/accreta: 60 capture(s) creado(s), 13 endpoint(s) reusaron uno existente
    /home/anibal/Workspace/accreta/subsystems/stratum: 8 capture(s) creado(s), 1 endpoint(s) reusaron uno existente
    150 archivo(s) afectado(s)

dry-run: no se escribió nada
```

Una misma migración aparece una vez por repo alcanzado. Cuando no hay nada pendiente:

```
$ bilinker migrate --recursive
ya aplicada: bilinker-001-capture-split
nada que migrar (4 capa(s) revisada(s))
```

### Código de salida de `migrate`

| Código | Condición |
|---|---|
| 0 | Migraciones aplicadas, o nada pendiente. |
| 1 | Error al aplicar una migración. Ninguna se registra en el ledger. |

## `bilinker restore-n1`

### Devuelve el vecindario que la `003` descartó, leyéndolo del backup del corte

```
bilinker restore-n1 [<path>] [--recursive] [--dry-run] [--from <dir>]
```

| Argumento | Descripción |
|---|---|
| `path` | Capa a restituir. Default: la capa actual. |
| `--recursive` | Alcanza también las capas descendientes encontradas en `.stratum/`. |
| `--dry-run` | Muestra qué escribiría sin escribir nada. |
| `--from <dir>` | De dónde leer el backup. Default: `.bilink-formato-3/` al lado del `.bilink/` de la capa. |

Escribe el contrato —`hash` y `hash_ast`— y declara con `link: unknown` que la ubicación no se recuperó, porque el backup no la tiene: los captures de los vecinos entraron al formato con la misma versión que los tiró. Es de un solo uso: existe porque una migración descartó un campo que tenía en la mano.

`--from` existe porque el backup puede no estar donde el corte lo dejó. No está en git, así que un clon fresco no lo trae y un árbol limpiado lo perdió; la copia que quedó es un tarball fuera de git.

### No es una migración, y la diferencia es verificable

Una migración tiene que ser reproducible desde lo que el repo contiene: dos personas que corren la misma migración sobre el mismo commit obtienen lo mismo, y eso es lo que hace que el ledger signifique algo. Esto lee un directorio que no está en git y que puede no existir, así que el resultado depende de qué tenga cada máquina. Un paso así no puede llevar un id en un registro que afirma qué le pasó al repo.

Y hay tres más, cada una suficiente:

| | |
|---|---|
| no cruza una versión de formato | los archivos son `4.1.0` válidos antes y después. Un ledger de migraciones de formato con una entrada que no mueve el formato deja de decir una sola cosa |
| no la necesitan todos los repos | sólo los que cortaron con la `003` teniendo vecindarios adquiridos. Una migración se registra igual en un repo donde no había nada que hacer |
| es condicional y parcial | saltea endpoints y tiene que decir cuáles. "La migración corrió" no distingue haber restituido todo de haber restituido la mitad |

La idempotencia, que es lo que el ledger habría dado gratis, sale de las condiciones: después de una corrida el `n` ya no es `declined`, así que la segunda corrida es no-op sin que nadie tenga que recordarlo.

### Un nivel se restituye sólo si las tres condiciones se cumplen

Fallar no es lo mismo en la primera que en las otras dos: la primera dice que no hay hueco que llenar, las otras dos que hay un hueco y no se puede. Sólo las segundas se cuentan como salteadas; contar las primeras sería reportar como pendiente cada endpoint del repo.

### El `accepted` vivo tiene el `n` en `declined`

Un `n` adquirido después del corte es más nuevo que el backup: alguien lo resolvió con un language server y eso es la verdad de hoy. Un `n` ausente dice que el fragmento no tiene firma resoluble, y el backup no puede contradecirlo. Sólo `declined` es el hueco que la `003` dejó, y es el único caso donde el backup sabe algo que el archivo vivo no. Que no se cumpla no es un salteo: es que ahí no había nada que devolver.

### El `hash` del fragmento no se movió

El discriminador está en el archivo: el `hash` del `accepted` vivo contra el del `accepted` del backup. El backup describe el código de antes del corte. Si el fragmento cambió desde entonces, ese vecindario era de otra versión de la firma y restituirlo afirmaría algo falso: un contrato aprobado para un fragmento que ya no es el que hay.

Esta condición se vence sola. Cada `accept` sobre un endpoint degradado le mueve el `hash`, y con eso el backup deja de aplicarle, sin que `accept`, `check` ni nada lo diga. La ventana no se cierra sólo por borrar el backup: se cierra por trabajar. Medido el 2026-09-02: ya había 8 endpoints así.

### Hay exactamente un `accepted`

Con más de una entrada el endpoint está `CONSENSUS_DIVERGED` y no hay un valor contra el cual comparar el `hash`. Elegir una sería resolver un desacuerdo entre personas leyendo un backup, que no es algo que un comando pueda hacer.

### Escribe el contrato y declara la ubicación como `unknown`

Del `accepted` del backup, donde el nivel era dos hashes sin `link`:

```yaml
      n:
        1:
          hash: d3fba7c71764b82cd17875f63ddba6864e015ddf8bc20adf62419ac44040327a
          hash_ast: 319cfc6fd8d01fa928f12876d9f8a93ff84b98ba92c0fdad3603118b04d2e816
```

Al `accepted` vivo, cuyo `n` decía `declined`:

```yaml
      n:
        1:
          link: unknown
          hash: d3fba7c7…
          hash_ast: 319cfc6f…
```

Los niveles se copian todos, no sólo el 1: si el backup tuviera un nivel 2 se restituye igual, con su propio `link: unknown`. Enumerar el 1 sería la clase de lista que envejece con el próximo nivel.

Y no toca nada más. Ni el `link` del endpoint, ni su `hash`, ni `agree`, ni la declaración de afuera. Lo único que cambia es el `n` de la entrada aceptada, el campo que la `003` reemplazó. Cada nivel restituido conserva su `hash_ast` o su ausencia, sin recalcular: el fold es todo-o-nada sobre los vecinos y este comando no tiene con qué recomponerlo.

### Es una decisión, y por eso la escribe esto y no `accept`

Restituir escribe adentro de `accepted`, que es territorio de `accept`. No es una excepción al reparto: el valor que se escribe es una decisión que alguien ya tomó —los hashes son la aprobación, con su `agree` intacto al lado— y esto la devuelve a donde estaba. No aprueba nada nuevo, y por eso no toca `agree`.

Lo que no hace es acuñar captures ni proponer ubicaciones. Eso es `apply`, y llenar los `unknown` es su propia tarea. Un vecindario restituido queda con la ubicación declarada como desconocida, nunca con captures adivinados.

### Un contrato se restituye de los dos lados, y se reporta de uno

Un endpoint `path` o `repo` lleva en su `accepted` una copia opaca del `accepted` del vecino, y esa copia incluye el `n`. Así que la `003` lo degradó dos veces por cadena —en el endpoint estructural y en la copia— y la restitución lo devuelve dos veces, cada capa desde su propio backup.

Y aun así se reporta una sola vez. El eje del vecindario es del endpoint estructural: `check` no lo evalúa sobre un `path` ni sobre un `repo`, donde lo que compara es la copia contra el vecino. La copia restituida coincide con lo que el vecino restituyó, así que el estado de la cadena queda limpio y el `CONTRACT_UNLOCATED` sale donde está el fragmento, que es donde alguien puede acuñar los captures. De ahí que contar endpoints y contar contratos den números distintos, y que el segundo sea el que dice cuánto trabajo hay.

### Los salteados van con su uuid y no sólo contados

La salida se agrupa por capa y nombra los que no pudo:

```
$ bilinker restore-n1 . --recursive

accreta
  restituidos  11
  salteados     4   el hash del fragmento se movió: el backup es de otra versión
    696c6d76.1  8b893c60.1  a5f2ebba.1  e671c12e.1

subsystems/bilinker/.stratum/impl
  restituidos  11
  salteados     4   el hash del fragmento se movió
    696c6d76.1  8b893c60.1  a5f2ebba.1  e671c12e.1

131 restituidos, 8 salteados, en 8 capas
```

Un endpoint salteado se queda en `declined`, y `declined` es una decisión: `check` lo reporta limpio y no vuelve a aparecer en ningún inventario. La salida de este comando es el único registro de que ahí había un contrato, así que el commit que lo corre tiene que poder nombrarlos.

### Los salteados no lo hacen fallar

| Código | Condición |
|---|---|
| 0 | La corrida terminó. Restituyó lo que las condiciones permitían y dijo qué salteó. |
| 1 | No pudo leer el backup o no pudo escribir un bilink. Nada queda a medias. |
| 2 | La versión de formato de la capa no se entiende. No se restituyó nada. |

Los salteados no son un error de la corrida sino un hecho sobre el pasado, y el comando hizo lo único que podía hacer con ellos: decirlo. Salir con 1 los volvería indistinguibles de un backup ilegible, que sí se puede arreglar. `--dry-run` no escribe nada; la segunda corrida no restituye nada, porque la primera condición ya no se cumple; y un endpoint que no cumple las condiciones se queda exactamente como estaba.

### `restore-n1` se retira cuando no queda backup que diga otra cosa

Cuando ningún repo alcanzable tenga un `accepted` con `n: declined` cuyo backup del corte diga otra cosa, o cuando ya no queden backups del corte, que es lo mismo desde el lado de esta herramienta. Es la misma asimetría de `bilinker-001-capture-split`: el código de un paso de un solo uso se va, y lo que queda es el registro de que corrió. Acá ese registro no es una entrada de ledger, porque esto no es una migración, sino los commits que lo corrieron, con los uuids salteados adentro.
