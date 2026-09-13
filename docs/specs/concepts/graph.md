# El grafo

`bilinker graph` recorre el grafo de bilinks a partir de un archivo, un fragmento o un UUID y muestra todos los nodos conectados, cruzando capas. Responde a la pregunta de con qué está vinculado algo, y a través de qué caminos. Es navegación: no modifica nada.

## El selector

### Un selector es un archivo, una posición, un UUID o toda la capa

```
bilinker graph <selector>
  [--depth <n>]
  [--format <tree|flat|json>]
  [--recursive]
```

| Selector | Comportamiento |
|----------|----------------|
| `archivo.md` | Todos los bilinks que referencian ese archivo en la capa actual |
| `archivo.md:42:5` | Bilinks cuyo capture cubre esa posición |
| `<uuid>`, ocho o más caracteres hexadecimales | Un bilink concreto, por UUID o prefijo |
| `.` o `*` | Todos los bilinks de la capa actual; con `--recursive`, los de todas las capas bajo la raíz del proyecto |

`--depth <n>` limita la profundidad del recorrido, y sin él no hay límite. `--depth 1` muestra sólo los bilinks directamente conectados al selector. Un selector que no resuelve a ningún archivo ni bilink conocido sale con 1; un error de lectura, con 2.

## El recorrido

### El traversal es un BFS que cruza capas por los endpoints `path`

Cada fragmento de archivo es un nodo; los endpoints `path` son aristas hacia otras capas.

```
graph(selector):
  1. Resolver selector → lista de bilinks iniciales
  2. Para cada bilink:
       emitir fragmento(s) estructural(es)
       para cada endpoint path no visitado:
         adjacent = stratum::resolve(path)
         si adjacent/.bilink/<uuid>.yaml existe: encolar
  3. Deduplicar por (UUID, raíz de capa, línea de inicio): fragmentos distintos
     del mismo archivo son nodos separados.
```

Usa el índice de la capa si está al día, y si no cae al escaneo del directorio.

### Si la capa adyacente no está clonada, el traversal se detiene sin fallar

Si el `.bilink/<uuid>.yaml` de la capa adyacente no existe localmente, el recorrido se detiene ahí en silencio. El nodo actual se muestra con su endpoint `path`, y el comando no falla por eso.

### Una cadena termina en un endpoint que no lleva a ningún lado más

Son tres, y el traversal se detiene en los tres:

| Terminador | Qué es | Cómo se emite |
|---|---|---|
| tip estructural | un fragmento de esta capa | el nodo, con su rango |
| `abstract` | una punta abierta a quien la consuma | un nodo sin destino, estado `OPEN` |
| repo | un fragmento de otro proyecto | una arista hacia el alias, sin cruzarla |

### El traversal no cruza la frontera

Un endpoint repo se emite como arista y se detiene ahí, aunque el clon del proveedor esté: seguirla significaría recorrer el grafo de otro proyecto, y el consumidor no sabe, ni tiene por qué saber, cuántas capas tiene el proveedor del otro lado.

### `abstract` no es un traversal que falló

Es una punta que nunca va a tener contraparte en su propio repo: quien la consume vive en otro proyecto que este repo no conoce. Emitirla como nodo con estado `OPEN` la distingue de una capa que no se pudo alcanzar, que es lo que un hueco confundiría.

### Un par (UUID, capa) ya visitado se muestra y no se recorre de nuevo

Si el traversal encuentra un par ya visitado lo muestra con `[ya visitado]` y no continúa por ahí.

## Los formatos

### `tree` es el formato por defecto

```
$ bilinker graph commands/pull.md

commands/pull.md
│
├── c0feab23  [OK ↔ OK]
│   │  link.0  commands/pull.md
│   │  link.1  >impl
│   │
│   └── c0feab23  [OK ↔ OK]  (.stratum/impl)
│       │  link.0  <
│       │  link.1  crates/stratum-cli/src/main.rs :: (enum_item ...) @target
│       │
└── b95021d2  [OK ↔ OK]
    └── b95021d2  [OK ↔ OK]  (.stratum/impl)
        │  link.1  crates/stratum-cli/src/main.rs :: (function_item ...) @target
        │
```

### `flat` es una línea por nodo

Para scripting:

```
$ bilinker graph commands/pull.md --format flat

c0feab23  OK ↔ OK  commands/pull.md  →  >impl  [.]
c0feab23  OK ↔ OK  <  →  crates/main.rs :: (enum_item ...)  [.stratum/impl]
```

### `json` es el contrato de proveedor hacia lattice

Emite las aristas de bilinker en el modelo de aristas de lattice, con los nodos ya resueltos a forma canónica. Es la forma en que bilinker actúa como proveedor: la resolución de una cadena a través de capas la hace bilinker, porque la topología es conocimiento de su formato; componerla con aristas de otros proveedores es tarea de lattice.

```json
[
  {
    "from":      ".::commands/pull.md#312~358",
    "to":        ".stratum/impl::crates/stratum-cli/src/main.rs#245~389",
    "kind":      "bilink",
    "guarantee": "accepted",
    "provider":  "bilinker",
    "directed":  false,
    "ref":       "c0feab23-1b2c-4d5e-8f6a-7b8c9d0e1f2a",
    "state":     ["OK", "OK"]
  }
]
```

`state` lleva la tupla de estados de los dos tips. Los `kind` emitidos son `bilink` y `task`, los dos con garantía `accepted`. `governs` no se emite: exige el endpoint de tipo bilink, que está especificado y no implementado.

Los formatos `dot` y `html` son de `lattice graph`, no de éste. Recorrer una cadena es conocimiento del formato bilink; renderizar el grafo nunca lo fue, y en lattice el visor muestra además las aristas de los otros proveedores.

### Una cadena de N nodos emite una arista entre sus dos tips estructurales

No N-1 aristas entre nodos `.bilink`. Los mids son mecanismo interno de bilinker, no conexiones del proyecto.

## Invariantes

1. `graph` nunca modifica ningún archivo.
2. Fragmentos distintos del mismo archivo generan nodos separados, identificados por su línea de inicio.
3. `--depth 1` muestra sólo los bilinks directamente conectados al selector.
