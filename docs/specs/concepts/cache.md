# La cache

Todo lo que se puede reconstruir de los demás archivos vive en `.bilink/cache/state`, fuera de git.

El criterio es uno solo: si un valor se puede recalcular, no va en el bilink. Lo que queda versionado es lo que nadie puede reconstruir: la declaración (`link`) y la decisión (`accepted`).

## Qué contiene

### Un archivo por capa, con lo derivable

```
.bilink/
  cache/
    state        ← un archivo por capa
```

| Campo | De quién | Qué es |
|---|---|---|
| `range` | del capture | Dónde cayó el fragmento en la última resolución: un tramo `start~end` por `@target`, separados por coma. |
| `state` | del capture | Si la ubicación resuelve. |
| `state.N` | del bilink | Si lo que hay coincide con lo aceptado, por endpoint. |
| `commit` | del bilink | En qué commit el fragmento quedó con el contenido aceptado. |
| `alias` | del bilink | Cómo se llama el fragmento en el vocabulario de su generador: `GET /public-api/user/info/from-token`. Sólo donde el endpoint declara [`as`](bilink.md). |

Es un archivo por capa, igual que `index/index`: reescritura atómica, menos inodos, y cero conflictos de merge porque no está versionado.

### `alias` está acá y no en el bilink

Guardado sería inerte, así que el día que alguien cambie la ruta el rótulo seguiría diciendo lo viejo y nada podría detectarlo: un rótulo falso sobre una referencia verificada es peor que no tener rótulo. Derivado no puede mentir: se compone del fragmento de hoy.

Que se derive no quiere decir que se recalcule en cada lectura. Componerlo exige resolver el capture, que es exactamente lo que `check` ya hace para escribir `range`: sale del mismo trabajo y lo escribe el mismo comando.

Va por endpoint y no por capture, aunque se componga del fragmento: la receta con la que se compone es el `as`, y el `as` es de una punta. Dos endpoints sobre el mismo capture pueden nombrarlo distinto, o uno nombrarlo y el otro no.

## Que no esté versionado hay que declararlo

### La regla vive en `.bilink/.gitignore`

Nada impide commitear un derivado; que no se commitee es una regla, y la regla vive en `.bilink/.gitignore`:

```
cache/
index/
```

Adentro de `.bilink/`, no en el `.gitignore` del repo ni en `.git/info/exclude`. Adentro viaja con el directorio que gobierna: una capa nueva en cualquier repo trae su regla puesta, y un clon fresco la tiene sin que nadie la configure. `info/exclude` es por clon —el segundo desarrollador no la hereda—, y el `.gitignore` del repo obligaría a una entrada por capa, que es una lista que se desincroniza con las capas que existen.

### La escriben los comandos que crean los directorios

En la misma operación: `check` al escribir la cache, `index` al escribir el índice. Que la regla sea un paso aparte es lo que la vuelve olvidable, y de hecho se olvidó: en el corte a formato 2 las cinco capas quedaron con `cache/state` versionado hasta que alguien lo miró.

Un `.gitignore` que ya exista se respeta: se agregan las entradas que falten y no se toca lo demás.

## Estar fría es normal

### Dos clases de derivado con garantías distintas

La cache no está en git, así que no tenerla es un estado corriente y no una anomalía: un clon fresco, un cambio de rama, después del corte de formato, `rm -rf .bilink/cache/`, u otra máquina.

| | Cache fría significa | Quién la escribe |
|---|---|---|
| `state` · `state.N` · `range` | no disponible: hay que correr `check` | `check` |
| `commit` | más lento, nunca no disponible: se re-deriva a demanda | `accept`; re-derivable por quien lo necesite |

Por eso que `commit` viva acá no bloquea a nadie. Donde hace falta —recuperar el texto aceptado, atribuir un cambio a un commit— sólo corre sobre endpoints ya no-OK, así que el costo está acotado por lo que está roto y no por el total.

### Cómo se re-deriva

Un walk acotado hacia atrás por la historia del archivo: desde HEAD, commit por commit, resolver la query contra esa versión del archivo y hashear el fragmento. El primero cuyo hash coincida con `accepted.hash` es el commit buscado.

No sirve `git log -L <inicio>,<fin>:<archivo>`: eso encuentra cuándo esas líneas quedaron como están ahora, y lo que se busca es dónde el fragmento tenía el contenido aceptado, que en un endpoint con drift es otro commit, y probablemente otras líneas.

Acotado por dos lados. Sólo se pregunta por endpoints ya no-OK, así que el costo lo fija lo que está roto y no el total; y el walk tiene un techo de commits, porque un `accepted.hash` de un fragmento que nunca existió en esta rama haría recorrer la historia entera para contestar que no. Al llegar al techo la respuesta es *"no lo encontré"*, y quien preguntó degrada, no falla.

Y se camina [la ref](ref.md), no la rama. Es lo que vuelve cierto que la ref protege también a la derivación, y no sólo al `commit` guardado: la ref alcanza todo commit del proyecto alguna vez absorbido, así que su historia es un superconjunto de la de la rama. Y como cada commit de la ref lleva el árbol del proyecto adentro, los paths son los mismos.

Lo que hace falta cubrir no es un rebase. Un rebase a secas preserva el contenido: el fragmento aceptado aparece igual en el commit reescrito, y el walk lo reencuentra ahí sin ayuda de nadie. Lo que rompe es un squash o un `filter-branch`, donde el contenido intermedio deja de existir en ningún commit de la rama, y ahí el único lugar donde sigue estando es la ref.

En un repo que todavía no cortó se camina `HEAD`, que es lo único que hay.

### `commit` lo escribe `accept`, y sigue siendo derivado

Es raro que un derivado lo escriba `accept` y no `check`, y es deliberado: se calcula una vez, en el momento en que hay todo el contexto a mano. Lo que define un derivado es que se pueda reconstruir, no quién lo escribió.

## Sabe a qué rama corresponde

### La cache registra el commit de la ref del que salió

`cache/state` registra el commit de `refs/bilink/<branch>` del que salió.

Sin eso, una cache por capa devolvería estados de otra rama en silencio cuando el árbol de trabajo cambia de rama. Con el commit anotado la cache se invalida sola: si no coincide, no sirve, y hay que correr `check`.

Es la mitad derivada de un par. La otra es [`.bilink/head`](ref.md), que dice a qué rama y a qué commit corresponde el `.bilink/` del árbol. La cache protege al derivado; `head` protege a la fuente, y por eso `head` no puede vivir acá adentro.

Y el commit lo lee de `head`, no de git. `head` es un hecho sobre el árbol, y es exactamente la pregunta que la cache necesita contestar: *"¿de qué commit salieron los bilinks sobre los que calculé?"*. Preguntárselo a git daría el tip de la ref, que es otra cosa: el árbol puede corresponder a un commit anterior. En una capa sin `head` no hay nada contra qué comparar y la cache se usa como está.

Lo que pasa sin esta invalidación no es un reporte equivocado, es una decisión perdida. `accept` le cree a la cache: si dice `OK`, no hay nada que aceptar y no escribe. Una cache de otra rama contestando `OK` sobre un fragmento que en ésta sí cambió deja el `accepted` viejo en su lugar, sin error, sin línea en ningún reporte, y con el `check` siguiente confirmando la mentira.

## Por qué `resolved_at` no está

### `resolved_at` se fue del formato, no se mudó

Tenía un solo uso funcional —ser el baseline de `git log --since=<resolved_at>` para atribuir un cambio a un commit— y ese uso está dominado por `commit`. Un timestamp es un mal baseline para arqueología en git: las fechas se desordenan con rebases, cherry-picks y skew de reloj, mientras que `git log <commit>..HEAD -- <file>` recorre por ancestría y es exacto.

Y de paso desapareció un defecto visible: `check` escribía `resolved_at` en cada corrida, así que ensuciaba archivos versionados sin cambio semántico.

## Invariantes

1. Todo lo que está en la cache es reconstruible sin ella.
2. La cache nunca es fuente de verdad. Si contradice a los archivos versionados, gana el archivo.
3. `check` es el único que escribe `state`, `state.N` y `range`.
4. La cache registra el commit de `refs/bilink/<branch>` al que corresponde, y se descarta sola si no coincide.
5. La cache no se versiona.
