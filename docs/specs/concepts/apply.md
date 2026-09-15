# Los fixes

`apply` corrige dónde apunta un endpoint cuando el fragmento se movió. Calcula el fix en el momento re-resolviendo contra git y el AST actuales: nunca reutiliza un cálculo previo.

`apply` acuña el [capture](capture.md) de la ubicación nueva y repunta el `link` del endpoint. No escribe `accepted`, así que el endpoint no queda `OK`: queda en `RELOCATED` hasta que alguien apruebe la ubicación nueva.

## `apply` propone, `accept` dispone

### Mover un vínculo a otro fragmento es una decisión

El fragmento nuevo puede no ser el que la spec describe, y una decisión sin aprobar es trabajo pendiente, no trabajo hecho. Por eso ningún fix devuelve un endpoint a `OK`.

Requiere git como dependencia dura.

### `apply` toma `--dry-run`, `--filter` y `-y`

```
bilinker apply [<uuid>[.<N>]] [--dry-run] [--filter <estado>] [-y]
```

| Flag | Descripción |
|---|---|
| `--dry-run` | Muestra los fixes que se aplicarían sin escribir nada. |
| `--filter <estado>` | Aplica sólo fixes de un estado específico (e.g. `--filter MOVED`). |
| `-y` | Omite la confirmación interactiva. |
| `<uuid>[.<N>]` | Aplica sólo los fixes de ese bilink, o de ese endpoint. El uuid va entero o por prefijo. |

### Un endpoint acota los fixes a ese bilink

Sin argumento, `apply` propone los fixes de toda la capa. Con un `<uuid>`, sólo los de ese bilink; con `<uuid>.<N>`, sólo los de ese endpoint. Lo demás no se mira como fix ni se escribe, y `--filter` se aplica encima.

Es lo que hace falta para repuntar el vecindario de un endpoint recién aceptado sin tocar el de los demás: con proveedor, `apply` propone además subir la cobertura de cada endpoint con el vecindario renunciado ([§ `apply` mantiene también `n.1.link`](#apply-mantiene-también-n1link)), y esa es una decisión de capa, no del endpoint que se está cerrando.

### `apply` re-deriva cada estado con fix, acuña el capture nuevo y repunta el `link`

0. Comprobar que la capa tenga estado calculado. Si no, fallar con 3 y nombrar `check` (ver "La capa tiene que estar mirada").
1. Escanear los bilinks de la capa actual.
2. Para cada endpoint en un estado con fix, re-resolverlo con el mismo algoritmo que usa [`check`](check.md). Eso produce un estado re-derivado y la ubicación actual del fragmento.
3. Calcular la ubicación nueva. Si coincide con la que el `link` ya tiene, es un no-op y se omite.
4. Mostrar el resumen y pedir confirmación (o `-y`).
5. Para cada fix: acuñar el capture de la ubicación nueva —si no existía— y repuntar el `link`.
6. Si el commit del proyecto contra el que se calcularon los fixes no está absorbido, absorberlo en un commit propio sobre [`refs/bilink/<branch>`](ref.md): un merge que sólo trae código.
7. Cerrar cada fix con un commit sobre la ref de un solo padre: un commit sobre la ref hace una cosa, y absorber y repuntar son dos. En un repo que todavía no cortó a la ref, los archivos escritos quedan en el árbol para que los commitee quien trabaja.

### Un commit por `link` repuntado, no por invocación

Un `apply -y` que corrige tres endpoints absorbe una vez —el paso 6— y escribe tres commits encadenados sobre ese merge. Es la misma granularidad por objeto que `accept`, y por el mismo motivo: repuntar un vínculo a otro fragmento es una decisión, se firma sola y se audita sola.

Y lo fuerza [el mensaje](ref.md): `apply <uuid>.<N> <capture-nuevo>` nombra un endpoint, y un mensaje que nombrara tres no sería reproducible contra el árbol de un solo padre.

`apply` y `accept` son dos commits porque son dos actos, con dos autores posibles: `apply` describe —repunta los `link` y deja los endpoints en `RELOCATED`— y `accept` bendice. El commit de `apply` no toca ningún `accepted`.

Mensaje de commit, uno por fix:

```
apply 7f3d8e9a-….1 3ca90f81…: MOVED → specs/domain/voting.yaml

Invocation: bilinker apply -y
Bilinker-Version: 0.1.0
```

El segundo argumento es el capture nuevo: es lo que el `link` pasa a nombrar, y lo que un replay compara. `Invocation:` guarda lo que la persona tipeó y es dato de auditoría, no de verificación.

## Cálculo del fix por estado

### Cada fix produce un `(file, query)` nuevo

| Estado | Cómo se encuentra la ubicación nueva |
|---|---|
| MOVED | El índice de renames de git: `git diff -M --name-status`. Se verifica que la query resuelva en el destino. |
| REANCHORED | La query relajada matchea un nodo con nombre distinto, por encima del umbral y con margen sobre el segundo. |
| CONTRACT_UNLOCATED | Se le pregunta al proveedor qué vecinos alcanza la firma. No hay conjunto declarado contra el que comparar, así que cualquiera que alcance es el fix. |

Los dos primeros producen lo mismo: un `(file, query)` nuevo, y con él un capture nuevo. Los criterios de detección son los de [check.md](check.md).

Ningún estado de aceptación tiene fix. `apply` corrige dónde está el fragmento, y eso lo dice el capture; que el contenido coincida con lo aprobado es una decisión, y las decisiones las escribe `accept`.

### No hay fork, porque no hay mutación

Un capture es inmutable y su id es el hash de su ubicación, así que corregir una ubicación siempre produce un capture distinto. `apply` lo acuña y repunta un solo `link`: el del endpoint que está corrigiendo. Los demás referentes no se enteran, sin que haya que decidir nada.

No hay copy-on-write, no hay tabla que decida por tipo de fix si forkear, y no hace falta contar referentes antes de aplicar: todos acuñan.

El capture viejo queda: sigue vivo mientras algún `accepted.link` lo nombre —es la ubicación que alguien aprobó— y lo limpia `capture prune` cuando ya no lo alcanza nadie.

### El estado se re-deriva; la cache no decide

`apply` no lee el estado cacheado. El que escribió el último `check` describe el árbol de ese momento, y el archivo pudo cambiar después: aplicar un fix derivado de esa foto es corregir contra algo que ya no está.

Así que re-resuelve el capture contra el árbol actual y decide con eso. De la cache sólo sale `commit`, que es un dato de git y no una conclusión sobre el estado del árbol. No hay dos valores que comparar, así que no puede quedar desincronizado.

### La capa tiene que estar mirada, y eso sí es un prerequisito

`apply` no corre sobre una capa fría: se planta, y el mensaje nombra el comando que la llena.

No contradice lo de arriba, porque son dos cosas distintas. Lo que `apply` se niega a heredar es una conclusión: que un endpoint esté `MOVED` lo re-deriva él, contra el árbol de ahora. Lo que exige es que la capa se haya mirado alguna vez, y eso no lo puede producir solo: el eje del vecindario arranca del rango del fragmento, y sin rango no hay posición que pasarle al proveedor. Un `apply` sobre una capa que nadie verificó no es un `apply` que no encuentra nada: es uno que no llegó a preguntar.

```
$ bilinker apply --dry-run

error: la capa no tiene estado calculado — 98 bilinks sin mirar.
  El vecindario se pregunta desde el rango del fragmento, y ese rango
  todavía no se derivó.

  Correr primero:  bilinker check .

exit 3
```

Se planta en vez de calentarla sola porque llenar la cache es el trabajo de `check`, y hacerlo acá lo escondería: el que adopta se comería el costo de verificar 98 bilinks adentro de un comando que dice *"propone fixes"*, sin haberlo pedido y sin saber que lo pagó.

Y la capa fría no es un caso raro: es el estado de todo clon nuevo, toda rama nueva y toda máquina nueva. Que ese camino termine en un error con el comando adentro, y no en un *"no hay nada que arreglar"*, es la diferencia entre un paso más y una conclusión falsa sobre el repo propio.

### Con la capa mirada queda el caso de a uno, que tampoco es "no hay nada"

El prerequisito cubre la capa entera; adentro, un endpoint suelto puede seguir sin poder mirarse: el capture no resuelve, y entonces no hay rango desde donde preguntar por su vecindario.

Ese endpoint no se cuenta como revisado. Son tres cosas distintas y sólo dos son ausencia de trabajo:

| Lo que pasó | Qué es |
|---|---|
| el fragmento no tiene vecindario alcanzable | no hay nada que arreglar |
| el conjunto de hoy coincide con el declarado | no hay nada que arreglar |
| no se pudo ubicar el fragmento para poder preguntar | no se miró |

Las dos primeras son una respuesta sobre el árbol. La tercera es la falta de una, y colapsarlas hace que el resumen final afirme algo que nadie verificó.

Así que el resumen los lista aparte, con el motivo por endpoint, y el código de salida no es 2: no es que no haya fixes disponibles, es que sobre esos no se sabe.

### Con una excepción: el endpoint que esta misma corrida está repuntando

Un endpoint en `MOVED` tampoco tiene rango —su capture no resuelve, que es justamente lo que lo puso en `MOVED`— y aun así no es un agujero: es una espera. El vecindario se pregunta desde el rango del fragmento, y el rango que vale es el de la ubicación nueva, que este mismo `apply` está proponiendo dos renglones más arriba.

Sin la excepción, cada `MOVED` produciría un *"no se sabe"* al lado del renglón que dice que se arregló. Un agujero que aparece siempre deja de leerse, y entonces el que importa pasa desapercibido.

La distinción es entre no poder preguntar y no poder preguntar todavía. El primero se reporta porque nadie lo va a resolver solo; el segundo lo resuelve el fix de al lado, y el `check` siguiente pregunta contra la ubicación nueva.

## Cuando el fix no se puede calcular

### Cuando no puede, lo dice y sigue

Un capture en `MOVED` o `REANCHORED` no siempre produce una ubicación nueva: git puede no reportar el rename, el anchor puede no localizarse, la query puede no tener predicado de nombre que reescribir.

Abortar dejaría sin revisar a todos los demás endpoints por culpa de uno.

### El caso que sí ve: MOVED y REANCHORED a la vez

Un archivo que se renombra y el símbolo capturado adentro también. Ningún estado lo expresa —los dos son de resolución y el capture guarda uno solo—:

```
warn: 36f0c759… endpoint.1: MOVED: el archivo se movió a 'crates/bilinker/src/issue.rs',
      pero el anchor `resolve_task_path` ya no está ahí (UNANCHORED).
      Repuntar con `bilinker recapture`.
```

El anchor se nombra, no el estado: es el dato que dice qué buscar.

Que no haya auto-fix está bien: dónde quedó el fragmento adentro del archivo destino es una inferencia que `apply` no debería hacer sola. Lo que sí puede es decir qué comando la hace.

### Y las otras dos causas no son de `apply`

| Lo que pasó | Estado del capture | Quién lo explica |
|---|---|---|
| el destino está y el anchor se renombró | `MOVED` | `apply` |
| git no detectó el rename: el destino no está trackeado | sin fix | [`get`](get.md) |
| el fragmento no está en ninguna parte | sin fix | [`get`](get.md) |

`apply` sólo toca los estados con fix, así que las dos últimas ni siquiera llegan a su código: dejan el capture sin fix, y `apply` las saltea. Quien las explica es `get`, que es donde se pregunta *"¿qué pasó con este endpoint?"*. Y la del destino sin trackear se decide buscando el anchor entre los archivos sin trackear: un hecho, no una sugerencia genérica.

## Qué escribe

### `accepted` no se toca nunca

| Archivo | Qué |
|---|---|
| `capture/<id>.yaml` | El capture de la ubicación nueva, si no existía. |
| el bilink | `link` del endpoint corregido, y nada más. |
| `cache/state` | El estado re-derivado tras el fix: `RELOCATED`. |

Es la invariante que separa proponer de aprobar, y con el bloque aparte es casi imposible de violar por accidente.

### La salida de `apply` lista los fixes y lo que no pudo mirar

```
$ bilinker apply

Pending fixes (3):
  MOVED      7f3d8e9a…  endpoint.1  → specs/domain/voting.yaml
  REANCHORED 3a4b5c6d…  endpoint.0  → anchor check_endpoint  (similitud 83%)

Sin mirar (1):
  b1c2d3e4…  endpoint.1  el capture no resolvió — no hay rango desde donde
                          preguntar por el vecindario

Apply? [y/N] y

Repuntados 3 endpoint(s). Los 3 quedan en RELOCATED.
  Revisar con `bilinker get <uuid>.<N>` y aprobar con `bilinker accept --place`.
commit:  refs/bilink/… @ 9f7020e  (absorbe 24ae0f6)
commit:  refs/bilink/… @ a4b5c6d
commit:  refs/bilink/… @ 3e1f8b2
commit:  refs/bilink/… @ c70d914
```

Ningún fix cierra solo. Por eso el resumen dice qué falta antes de listar los commits: el trabajo no terminó cuando `apply` termina, y los tres endpoints quedaron esperando un `accept --place`.

El bloque de "sin mirar" va arriba, entre los fixes y la confirmación, y no al final con los commits: es lo que la persona necesita para decidir si el resumen le alcanza, y un renglón después de la lista de commits ya no se lee. Cada línea dice el motivo por endpoint, porque *"no se pudo"* sin decir qué falló es la misma respuesta vacía en otro lugar.

### Código de salida de `apply`

| Código | Condición |
|---|---|
| 0 | Todos los fixes aplicados. |
| 1 | Error al calcular o aplicar algún fix, o algún endpoint que no se pudo mirar. |
| 2 | No hay endpoints con fix disponible, y todos se miraron. |
| 3 | La capa no tiene estado calculado: falta correr `check`. |

El 2 es una afirmación sobre el árbol, así que sólo sale cuando hubo con qué hacerla. Un endpoint que no se pudo ubicar la debilita, y por eso cae en el 1 junto con los demás casos de *"no se sabe"*. El 3 no es un caso de eso: es el prerequisito sin cumplir, y se distingue porque lo arregla otro comando.

## El vecindario, para lo cual recibe el puerto

### `apply` mantiene también `n.1.link`

`apply` repunta `link` cuando el fragmento se movió. Con el vecindario siendo [captures](accept.md), repunta también `n.1.link`, que es la misma operación N veces: un vecino cuyo archivo se renombró es `MOVED`, y eso lo resuelve git.

Pero el conjunto no sólo se mueve: gana y pierde miembros.

```java
- public Dto get(String t)
+ public Dto get(String t, Filtro f)
```

De `{Dto}` a `{Dto, Filtro}`. No es un `MOVED` ni un `REANCHORED`: es un miembro nuevo, y qué tipo es `Filtro` sólo lo sabe un language server. Así que `apply` recibe el proveedor de vecindario, igual que `check` y `accept`.

Sin proveedor arregla lo del fragmento y dice que no pudo tocar el vecindario. Todo comando que toque el eje del vecindario recibe el puerto, y degrada sin él. La frontera del subsistema no se mueve: la librería sigue siendo git y tree-sitter, y el proveedor entra por el puerto.

### Y llena un `unknown`, que es el otro modo de ganar miembros

Un nivel con [`link: unknown`](bilink.md) es *"el contrato está y de qué vecinos salió no se sabe"*. Preguntarle al proveedor qué vecinos alcanza la firma es el fix, y es la misma operación que ganar un miembro: se descubre un conjunto y se propone.

La diferencia con `{Dto}` → `{Dto, Filtro}` es de qué se compara contra qué. Ahí el conjunto declarado existía y le faltaba uno; acá no hay con qué comparar, así que cualquier conjunto que el proveedor alcance es el fix. `unknown` es incomparable por definición: dos `unknown` no coinciden, y tampoco coincide con una lista.

Y `apply` sólo llena la declaración. El `accepted` sigue con su `unknown` y su hash conservado, porque `apply` no escribe `accepted` nunca, así que el endpoint sigue en `CONTRACT_UNLOCATED` hasta que alguien acepte. Los captures propuestos se revisan con [`get`](get.md) antes de que se conviertan en un contrato.

El orden con el que se llena no es preferencia. Los captures salen de las posiciones que el recorrido de la firma le pasa al proveedor, así que llenar con esas posiciones mal calculadas escribe un capture que apunta al propio fragmento, y un `accept` encima lo vuelve permanente. `unknown` es un estado seguro y el verde equivocado no.

### El mensaje de un fix de vecindario nombra el capture del fragmento

La gramática de la ref es `apply <uuid>.<N> <capture>`, con un id de capture. Un fix de vecindario no tiene un capture nuevo: repunta un conjunto. El que va en el mensaje es el del fragmento cuyo vecindario se repuntó, el `link` del endpoint, y los vecinos van en la prosa:

```
apply 7f3d8e9a-….1 3ca90f81…: N1 n1 → 2 vecino(s): capture 38512d9f… c88409d6…
```

### Y sigue sin aprobar nada

Proponer un miembro nuevo del vecindario es una propuesta como cualquier otra: deja el endpoint en `RELOCATED` y no escribe ningún `accepted`. `apply --dry-run` con proveedor lo dice antes de tocar nada:

```
n1: la firma menciona `Filtro`, que no está declarado — se agregaría
```

Lo que `apply` no hace, y no debe: detectar que el tipo de retorno pasó de `A` a `B`. Eso está adentro del fragmento, así que ya disparó `ALTERED` con tree-sitter, y ningún estado de aceptación se arregla solo. Quien mira acepta, y `accept` re-deriva el vecindario.

## Invariantes

- `apply` nunca escribe `accepted`. Corrige dónde está el fragmento, nunca qué se aprobó.
- `apply` nunca modifica un capture existente: acuña.
- El único efecto de `apply` sobre un bilink es repuntar un `link`, del fragmento o del vecindario.
- Tras `apply`, el endpoint queda en `RELOCATED`. Ningún fix lo devuelve a `OK`.
- `apply` nunca aplica un fix derivado de la cache: cada uno se recalcula re-resolviendo contra el árbol y el índice git actuales.
- `apply` no corre sobre una capa sin estado calculado: falla con 3 y nombra `bilinker check .`. La cache no le da conclusiones; le da la prueba de que la capa se miró.
- Un endpoint que no se pudo ubicar no se cuenta como revisado: se lista aparte con su motivo, y no deja el código de salida en 2.
- Si el fix calculado no se puede verificar —el hash no coincide en el path nuevo, el anchor no aparece— `apply` lo rechaza y avisa.
- `apply` es idempotente: un fix ya aplicado se detecta como no-op.
- `apply` escribe un commit de decisión por `link` repuntado, todos hijos de la misma absorción.
- `ALTERED`, `UNRESOLVED` y `CHAIN_DIRTY` no tienen fix y `apply` no los toca.
- Un nivel del vecindario en `unknown` tiene fix sólo con proveedor: sin él, `apply` lo deja como está y lo dice. Y el fix llena la declaración, nunca el `accepted`.
