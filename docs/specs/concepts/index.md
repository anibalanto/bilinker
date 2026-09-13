# El índice

Encontrar todos los bilinks que referencian un archivo dado requiere escanear todos los bilinks de la capa: O(N). Cada capa puede tener un archivo `.bilink/index/index` que mapea archivos fuente a los endpoints de bilinks que los referencian, y la búsqueda pasa a O(1).

## El archivo

### El índice es opcional y regenerable

Si no existe o está desactualizado, los comandos que lo usan caen al scan O(N). Nunca es fuente de verdad: siempre puede reconstruirse a partir de los bilinks.

### Formato

```
docs/api-spec.md	7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.0	c1a2b3c4-…
src/Service.java	a3f9c821-4e5b-4c3d-9f2a-1b2c3d4e5f6a.1	d5e6f7a8-…
src/Service.java	7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.0	d5e6f7a8-…
```

- Una entrada por línea: `<archivo>\t<uuid>.<N>\t<capture-id>`, separados por tabulador.
- Dos entradas pueden compartir `<capture-id>`: es el caso de un capture referenciado por varios bilinks.
- Un mismo archivo puede tener múltiples entradas.
- Las líneas que comienzan con `#` son comentarios y se ignoran.
- Encoding UTF-8 sin BOM.

La ruta del archivo es relativa a la raíz de la capa que contiene el `.bilink/`. El resto de la información se lee de donde vive: `file` y `query` en el capture, `accepted` en el bilink, y `range` y `state.N` en [la cache](cache.md).

### Ubicación

```
<layer-root>/
  .bilink/
    index/
      index             ← índice bilinker de esta capa
    7f3d8e9a-….yaml
    a3f9c821-….yaml
```

Cada capa tiene su propio índice. El índice solo cubre los endpoints estructurales de los bilinks que viven en esa capa: no endpoints `path` ni bilinks de otras capas.

### Detección de obsolescencia

El índice se considera válido si su mtime es mayor o igual al mtime del bilink más reciente en el mismo directorio. Si algún bilink es más nuevo que el índice, el índice está desactualizado.

Los comandos que usan el índice lo verifican antes de usarlo: índice válido, lookup O(1); índice ausente o desactualizado, scan O(N) sobre los bilinks de la capa.

El scan de fallback nunca regenera el índice automáticamente: eso es responsabilidad explícita de `bilinker index`.

### El índice no almacena ranges ni queries

El índice responde *"¿qué bilinks referencian este archivo?"* en O(1). Una vez recuperados los UUIDs relevantes, el `range` de cada capture se lee de la cache.

### No se versiona

Va en `.bilink/.gitignore` junto con `cache/` ([cache.md](cache.md), "Que no esté versionado hay que declararlo"): cada desarrollador construye su índice localmente.

## El comando `index`

### `bilinker index` construye o reconstruye el índice

```
bilinker index [<path>] [--recursive]
```

| Argumento | Tipo | Descripción |
|---|---|---|
| `<path>` | path | Capa o directorio raíz donde construir el índice. Por defecto: capa actual (cwd). |
| `--recursive` | flag | Construye el índice en todas las capas descendientes también. |

1. Localiza el directorio `.bilink/` en `<path>`.
2. Escanea todos los bilinks de esa capa.
3. Para cada endpoint estructural, lee su capture y extrae `(archivo, uuid, N, capture-id)`.
4. Escribe `.bilink/index/index` con una línea `<archivo>\t<uuid>.<N>\t<capture-id>` por entrada.
5. Si `--recursive`, repite para cada capa descendiente encontrada en `.stratum/`.

El índice generado reemplaza cualquier versión anterior: la operación es idempotente.

```
$ bilinker index --recursive

index: .bilink/index/index              (3 entradas)
index: .stratum/impl/.bilink/index/index   (7 entradas)
```

Con `--quiet`, solo imprime errores.

`bilinker get` usa el índice si está disponible y actualizado; si no, hace scan O(N) sin error. `bilinker index` es la única forma de construir o actualizar el índice: ningún otro comando lo escribe.

### `bilinker index status` reporta si el índice de cada capa está al día

```
bilinker index status [<path>] [--recursive]
```

Reporta si el índice de cada capa está actualizado, desactualizado o ausente, sin modificar ningún archivo.

```
$ bilinker index status --recursive

.bilink/index/index              OK        (actualizado)
.stratum/impl/.bilink/index/index          STALE     (2 bilinks más nuevos)
.stratum/impl/.bilink/index/index  MISSING
```

### Códigos de salida

| Código | Condición |
|---|---|
| 0 | Índice construido exitosamente. |
| 1 | Error de lectura/escritura en alguna capa. |

## Invariantes

1. El índice es un derivado: nunca modifica la fuente de verdad (los bilinks).
2. Un índice válido es consistente con los bilinks actuales de su capa.
3. La ausencia del índice nunca es un error: degrada a O(N) silenciosamente.
4. El índice no cubre endpoints `path`, solo endpoints estructurales.
