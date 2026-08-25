# Servidor FHIR Helios — catálogo de mensajes de la interfaz
# Configuración regional: Español (es)
#
# Mantenga las mismas claves que en `en/main.ftl` (la configuración regional
# de origen). Las claves que falten recurren a inglés según la cadena de
# reserva descrita en docs/multi-language.md.

## Marca / términos compartidos

-app-name = Servidor FHIR Helios
-org-name = Helios Software

## Estructura de página

app-title = { -app-name }
app-tagline = Un servidor FHIR rápido y multiversión

nav-dashboard = Panel
nav-terminology = Terminología
nav-terminology-new-window = Terminología (se abre en una pestaña nueva)
nav-resources = Recursos
nav-settings = Configuración
nav-signout = Cerrar sesión

## Selector de idioma

language-label = Idioma
language-en = Inglés
language-es = Español
language-de = Alemán

## Página de inicio

home-lede = Interfaz renderizada en el servidor y basada en HTMX. Este panel se actualiza como un fragmento HTML.

## Panel de estado

status-last-checked = Última comprobación: { $timestamp }

## Panel / estado

dashboard-heading = Panel del servidor
health-status-ok = Todos los sistemas operativos
health-status-degraded = Algunos sistemas están degradados
health-uptime = Tiempo activo: { $duration }

resource-count = { $count ->
    [one] { $count } recurso
   *[other] { $count } recursos
}

## Exploración de terminología

terminology-heading = Servidor de terminología
terminology-lede = Conecta HFS con un servidor de terminología FHIR.
terminology-configured-heading = Servidor de terminología configurado
terminology-configured-body = HFS_TERMINOLOGY_SERVER apunta a una URL válida.
terminology-configured-open = Abrir servidor de terminología
terminology-invalid-heading = HFS_TERMINOLOGY_SERVER no es válida
terminology-invalid-body = Usa una URL HTTP o HTTPS absoluta con un host. Se permiten rutas y una barra final. No incluyas credenciales, parámetros de consulta ni fragmentos.
terminology-invalid-note = Actualiza la variable de entorno y luego reinicia HFS.
terminology-setup-heading = Conectar un servidor de terminología
terminology-setup-body = Define HFS_TERMINOLOGY_SERVER con la URL base del servidor de terminología FHIR que debe usar HFS.
terminology-setup-note = Define la variable en el entorno desde el que se inicia HFS y luego reinicia el servidor.
terminology-search-label = Buscar CodeSystems y ValueSets
terminology-search-placeholder = p. ej. 73211009, «diabetes», http://snomed.info/sct
terminology-display-language = Idioma de visualización
terminology-no-results = No se encontraron conceptos coincidentes.

## Acciones comunes

action-search = Buscar
action-save = Guardar
action-cancel = Cancelar
action-retry = Reintentar

## Errores (refleja el texto de OperationOutcome; véase docs/multi-language.md §5)

error-not-found = No se encontró el recurso solicitado.
error-unauthorized = No está autorizado para realizar esta acción.
error-generic = Algo salió mal. Vuelva a intentarlo.

## Estructura del panel (Figma «Dashboard V1.1»)

nav-section-work = Trabajo
nav-section-batch-data = Lotes y datos
nav-section-server = Servidor
nav-section-conditional = Condicional

nav-home = Inicio
nav-search = Buscar
nav-resource-editor = Editor de recursos
nav-history-versions = Historial y versiones
nav-compartments = Compartimentos
nav-batch-transaction = Lote / Transacción
nav-import = Importar
nav-export = Exportar
nav-sql-on-fhir = SQL-on-FHIR
nav-capability-conformance = Capacidad y conformidad
nav-search-parameters = Parámetros de búsqueda
nav-admin-ops = Administración / Operaciones
nav-subscriptions = Suscripciones
nav-tenants = Tenants

## Mantenimiento de tenants (/ui/tenants)

tenants-title = Mantenimiento de tenants
tenants-unavailable = El registro de tenants no está disponible en este backend de almacenamiento.
tenants-stat-total = Tenants totales
tenants-stat-total-sub = { $count ->
    [one] { $count } registrado
   *[other] { $count } registrados
}
tenants-stat-resources = Recursos almacenados
tenants-stat-resources-sub = en todos los tenants
tenants-search-placeholder = Buscar por nombre o id de tenant…
tenants-add = Añadir tenant
tenants-add-title = Añadir un tenant
tenants-field-id = Id del tenant
tenants-field-id-hint = Se usa en la API (cabecera X-Tenant-ID, prefijo de URL, claim del JWT).
tenants-field-name = Nombre visible (opcional)
tenants-field-name-hint = Una etiqueta legible; no se usa para el enrutamiento.
tenants-add-submit = Aprovisionar tenant
tenants-col-tenant = Tenant
tenants-col-resources = Recursos
tenants-col-created = Creado
tenants-col-actions = Acciones
tenants-empty = Ningún tenant coincide.
tenants-unregistered = sin registrar
tenants-delete = Eliminar tenant
tenants-delete-confirm = ¿Dar de baja el tenant «{ $id }»? Sus datos se conservan salvo que se purguen vía API.
tenants-row-provisioning = Aprovisionando… puede tardar un momento.
tenants-row-failed = El aprovisionamiento falló
tenants-dismiss = Descartar

tenant-heading = Tenants
tenant-all = Todos los tenants
tenant-search-placeholder = Buscar tenants

theme-label = Tema
theme-light = Tema claro
theme-dark = Tema oscuro

fhir-version = FHIR { $version }
fhir-version-heading = Versión FHIR

card-resource-types = Tipos de recursos
card-resource-types-sub = habilitados para { $version }
card-stored-resources = Recursos almacenados
card-stored-resources-sub = en el tenant activo
card-export-jobs = Trabajos de exportación
card-export-jobs-sub = en ejecución ({ $queued } en cola)
card-import-jobs = Trabajos de importación
card-import-jobs-sub = activos
card-jobs-unavailable = no disponible
card-uptime = Tiempo activo
card-uptime-sub = desde el arranque del proceso

chart-title = Recursos FHIR en el tiempo
chart-window = Intervalo de tiempo del gráfico
chart-pick-heading = Tipos de recurso graficados
chart-pick-all = Ver todos los tipos de recurso
chart-pick-filter = Filtrar tipos
chart-empty = Nada que graficar todavía: los recursos almacenados aparecerán aquí a medida que se creen.
chart-sample-note = Datos de muestra: esta build no tiene registrado un proveedor de métricas en vivo.
chart-table-toggle = Ver como tabla
chart-table-when = Momento
chart-focus-series = Enfocar esta serie
chart-unfocus-series = Mostrar todas las series

## Pie de página

footer-copyright = © { $year } { -org-name }

## Historial y versiones (#236)

history-heading = Historial y versiones
history-lede = Compara dos versiones de un recurso. El almacenamiento está totalmente versionado; esto lo lee con la API estándar _history y vread.
history-type-label = Tipo de recurso
history-id-label = Id del recurso
history-id-placeholder = id del recurso
history-load = Cargar
history-tabs-label = Alcance del historial
history-tab-instance = Instancia
history-tab-type = Feed por tipo
history-tab-system = Feed del sistema
history-versions-label = Versiones
history-pick-instance = Elige una instancia
history-current = actual
history-from = Desde
history-to = Hasta
history-show-metadata = Mostrar cambios de metadatos
history-empty = Carga un recurso y elige dos versiones para comparar.
history-load-error = No se pudo cargar el historial de ese recurso.
history-not-found = No hay historial para ese recurso — revisa el tipo y el id.
history-diff-heading = { $from }
history-metadata-hidden = { $count ->
    [one] { $count } cambio de metadatos oculto
   *[other] { $count } cambios de metadatos ocultos
}
history-textual = Ver diff de texto completo
history-only-metadata = Entre estas versiones solo cambiaron los metadatos.
history-identical = Estas dos versiones son idénticas.
history-deleted = { $version } es una eliminación — no hay contra qué comparar.
history-parse-error = No se pudieron leer esas versiones como JSON.
## Saved queries (#234)

nav-saved-queries = Consultas guardadas

queries-heading = Consultas guardadas
queries-lede = Guarda consultas de búsqueda FHIR por tipo de recurso, ordenadas por su última ejecución. Se guardan en tu configuración de usuario y te siguen entre dispositivos.
queries-add-heading = Guardar una consulta
queries-type-label = Tipo de recurso
queries-type-placeholder = p. ej. Patient
queries-name-label = Nombre
queries-name-placeholder = p. ej. Smith en Boston
queries-query-label = Cadena de consulta
queries-query-placeholder = p. ej. name=smith&address-city=Boston
queries-empty = Aún no hay consultas guardadas. Guarda una arriba para empezar.
queries-never-run = Nunca ejecutada
queries-run = Ejecutar
queries-rename = Renombrar
queries-delete = Eliminar
queries-rename-prompt = Nuevo nombre
queries-confirm-delete = ¿Eliminar «{ $name }»?
queries-unavailable = Las consultas guardadas no están disponibles: el backend de almacenamiento de este servidor no admite configuración por usuario.

## Visor de SearchParameters (#238)

sp-heading = Parámetros de búsqueda
sp-lede = Explora los parámetros con los que este servidor resuelve las búsquedas, filtrados por tipo de recurso base. Los parámetros almacenados se pueden crear, editar y eliminar; el registro recoge los cambios por tenant.
sp-version-label = Versión FHIR
sp-spec-missing = No se encontró el bundle completo de la especificación (search-parameters-*.json) en el directorio de datos — solo se muestran los parámetros mínimos embebidos.
sp-rail-label = Filtro de recursos
sp-rail-search = Filtrar tipos
sp-rail-recent = Usados recientemente
sp-rail-types = Tipos de recurso
sp-rail-all = Todos los tipos
sp-facet-type = Tipo
sp-facet-type-label = Filtrar por tipo de parámetro
sp-facet-source = Origen
sp-facet-source-label = Filtrar por origen
sp-source-embedded = embebido
sp-source-stored = almacenado
sp-source-config = configuración
sp-chip-conflict = conflicto
sp-chip-overrides = anula la spec
sp-chip-shadowed = eclipsado
sp-col-code = Código
sp-col-type = Tipo
sp-col-base = Base
sp-col-expression = Expresión
sp-col-source = Origen
sp-total = { $count } parámetros
sp-pagination-label = Páginas
sp-page-prev = Anterior
sp-page-next = Siguiente
sp-detail-label = Detalle del parámetro
sp-detail-empty = Ningún parámetro seleccionado
sp-detail-empty-hint = Selecciona una fila para inspeccionar su definición, su expresión y cómo se resuelve en el registro.
sp-detail-readonly = Parámetro de la especificación (compilado desde el archivo de datos) — solo lectura.
sp-field-url = URL canónica
sp-field-name = Nombre
sp-field-status = Estado
sp-field-base = Tipos de recurso base
sp-field-expression = Expresión FHIRPath
sp-field-description = Descripción
sp-field-target = Tipos destino
sp-field-components = Componentes
sp-status-hint = El cargador promueve el estado draft de la especificación a active al cargar.
sp-note-conflict = (base, code) duplicado dentro del mismo origen que { $url } — el registro rechaza esta colisión (DuplicateCode).
sp-note-overrides = Anula a { $url } en (base, code): una definición almacenada tiene precedencia sobre el parámetro de la spec, así que esta resuelve las búsquedas. El registro emite un WARN con ambas URLs.
sp-note-shadowed = Eclipsado por { $url } en (base, code): un origen de mayor precedencia resuelve las búsquedas de este slot.
sp-note-empty-expression = Expresión vacía: el extractor no indexa ninguna fila, así que toda búsqueda con este parámetro devuelve vacío en silencio.
sp-note-no-target = Parámetro de referencia sin tipos destino: la búsqueda encadenada no puede resolver el tipo referenciado.
sp-note-choice-type = Expresión de tipo choice: el extractor reescribe ofType(T) / as T al elemento concreto (por ejemplo valueQuantity) antes de evaluar contra el JSON almacenado.
sp-new = Nuevo parámetro de búsqueda
sp-edit = Editar
sp-delete = Eliminar
sp-delete-confirm = ¿Eliminar este parámetro de búsqueda almacenado? Las búsquedas que lo usan dejarán de coincidir cuando el registro se actualice.
cmp-new = Nueva definición de compartimento
cmp-edit = Editar
cmp-delete = Eliminar
cmp-delete-confirm = ¿Eliminar esta definición de compartimento? Sus rutas de compartimento dejarán de resolverse.
crud-delete-failed = Error al eliminar

## Visor y probador de compartments (#237)

cmp-heading = Compartimentos
cmp-lede = Las definiciones de compartment con las que este servidor enruta las peticiones /{"{"}compartment{"}"}/{"{"}id{"}"}/{"{"}type{"}"}, y un probador que responde: ¿está este tipo en este compartment, mediante qué parámetros, y qué búsqueda ejecuta el servidor?
cmp-rail-label = Definiciones de compartment
cmp-rail-heading = Compartimentos
cmp-degraded = Las definiciones de compartimento no se pudieron cargar de este servidor en este momento — la auto-llamada a /CompartmentDefinition falló (con autenticación habilitada esto suele significar que el token de servicio saliente falta o es inválido). La página reintenta en la siguiente petición.
cmp-rail-note = Las definiciones son recursos almacenados, sembrados desde la especificación FHIR al arrancar. Las ediciones y eliminaciones aquí son por tenant.
cmp-tabs-label = Secciones del compartment
cmp-tab-definition = Definición
cmp-tab-members = Miembros
cmp-tab-tester = Probador
cmp-field-code = Código
cmp-field-status = Estado
cmp-field-url = URL canónica
cmp-field-version = Versión
cmp-field-publisher = Editor
cmp-field-description = Descripción
cmp-field-search = search
cmp-field-experimental = experimental
cmp-search-why = Apagado significaría que ninguna ruta de compartment resuelve para este compartment.
cmp-on = activado
cmp-off = desactivado
cmp-yes = sí
cmp-no = no
cmp-readonly-note = Solo lectura: estos valores provienen de las definiciones de la especificación compiladas en el servidor.
cmp-filter-members = Miembros
cmp-filter-all = Todos los tipos
cmp-filter-excluded = Excluidos
cmp-member = miembro
cmp-excluded = excluido
cmp-tester-id = Id
cmp-tester-target = Tipo destino (o *)
cmp-tester-run = Probar
cmp-result-member = ✓ miembro — vía { $params }
cmp-result-flat = // búsqueda plana equivalente
cmp-result-member-note = El servidor resuelve la ruta de compartment a esta búsqueda sobre los parámetros de referencia del tipo.
cmp-result-self = ✓ miembro — el propio recurso del compartment ({"{"}def{"}"})
cmp-result-self-note = La instancia del compartment está trivialmente en su propio compartment; la ruta lee el recurso directamente.
cmp-result-notmember = ✕ { $type } no es miembro de este compartment
cmp-result-notmember-note = El servidor devuelve 404 con un OperationOutcome para tipos que no son miembros del compartment.
cmp-result-fanout = Se expande a { $count } tipos miembro
cmp-result-fanout-note = Los tipos excluidos se omiten, no fallan — el fan-out descarta los tipos no miembro en lugar de dar error.
queries-builder-heading = Constructor de búsquedas
queries-url-label = URL de búsqueda FHIR
queries-url-placeholder = GET /Patient?name=smith&birthdate=ge1980-01-01
queries-builder-hint = Edita la URL GET directamente o mediante las filas de abajo — se mantienen sincronizadas. Ejecutar corre la búsqueda aquí mismo y la registra en Recientes; ponle un nombre para conservarla en la lista.
queries-recent = Recientes
queries-recent-heading = Búsquedas recientes
queries-recent-empty = Aún no hay búsquedas recientes — ejecuta una para registrarla aquí.
queries-invalid-url = Escribe una búsqueda como GET /Patient?name=smith — el tipo de recurso sale de la ruta.

queries-conditions = Condiciones
queries-add-condition = Añadir condición
queries-includes = Includes
queries-result-controls = Controles de resultado
queries-remove = Quitar
queries-match-is = es
queries-or = + o
plain-pill = En lenguaje claro
plain-find = Buscar registros de {"{type}"}
plain-clause = {"{path}"} {"{verb}"} {"{value}"}
plain-clause-no-value = {"{path}"} {"{verb}"}
plain-and = y
plain-or = o
plain-arrow = {" "}→
plain-has = que tienen un {"{type}"} relacionado cuyo {"{param}"} {"{verb}"} {"{value}"}
plain-has-no-value = que tienen un {"{type}"} relacionado cuyo {"{param}"} {"{verb}"}
plain-include = Devolviendo también el {"{param}"} de cada {"{type}"}{"{target}"}
plain-revinclude = Más cada {"{type}"} cuyo {"{param}"} apunta aquí
plain-iterate = (repetidamente)
plain-count = Mostrando {"{n}"} por página
plain-sort = Ordenado por {"{sort}"}
plain-verb-is = es
plain-verb-contains = contiene
plain-verb-exact = es exactamente
plain-verb-missing = está presente/ausente
plain-verb-missing-true = está ausente
plain-verb-missing-false = está presente
plain-verb-not = no es
plain-verb-text = coincide con el texto
plain-verb-in = está en el value set
plain-verb-not-in = no está en el value set
plain-verb-identifier = tiene el identificador
plain-verb-of-type = tiene un identificador de tipo
plain-verb-ge = es igual o posterior a
plain-verb-le = es igual o anterior a
plain-verb-gt = es posterior a
plain-verb-lt = es anterior a
plain-verb-ne = no es
plain-verb-eq = es
plain-verb-sa = comienza después de
plain-verb-eb = termina antes de
plain-verb-ap = es aproximadamente
queries-related-heading = Incluir datos relacionados
queries-related-sub = Añade recursos conectados a los resultados.
queries-related-add-include = Incluir un recurso al que apunta
queries-related-add-revinclude = Incluir recursos que apuntan aquí
queries-iterate = Iterar
queries-sort-label = Orden
queries-sort-default = Predeterminado
queries-sort-recent = Más recientes
queries-sort-oldest = Más antiguos
queries-sort-id = ID
queries-modify-heading = Modificadores
queries-mod-exact = valor completo, incl. mayúsculas y acentos
queries-mod-contains = coincide en cualquier parte del texto
queries-mod-missing = el campo está presente / ausente
queries-mod-text = tratamiento avanzado de texto
queries-mod-not = ningún valor coincide
queries-mod-above = este o un ancestro
queries-mod-below = este o un descendiente
queries-mod-in = miembro del value set
queries-mod-not-in = no es miembro del value set
queries-mod-identifier = compara la referencia por identificador
queries-mod-of-type = compara tipo, sistema y valor del identificador
queries-chain-into = Filtrar por una propiedad del recurso referenciado
queries-chain-any-target = cualquiera
queries-has-pill = tiene un recurso relacionado
queries-has-type-placeholder = tipo de recurso
queries-has-via = enlazado vía
queries-has-where = donde su
queries-add-has = ⧉ Filtrar un recurso que enlaza aquí
queries-param-placeholder = parámetro
queries-value-placeholder = valor
queries-results = Resultados
queries-results-total = { $count } resultados
queries-results-included = { $count } incluidos
queries-results-empty = Sin resultados.
queries-open-tab = Abrir en pestaña nueva
queries-col-updated = Actualizado
queries-prev = Anterior
queries-next = Siguiente

queries-rail-heading = Tipos de recurso
queries-rail-filter = Filtrar tipos

## Búsqueda — lenguaje natural y constructor visual (#255)

search-heading = Buscar
search-lede = Describe lo que buscas, o arma la consulta a mano. En ambos casos obtienes una búsqueda FHIR que puedes leer, corregir y ejecutar.
search-query-tag = CONSULTA
search-copy = Copiar la consulta

search-mode-label = Cómo escribir la consulta
search-mode-nl = Lenguaje natural
search-mode-builder = Constructor visual

search-nl-label = Describe la búsqueda
search-nl-placeholder = Describe lo que buscas — p. ej. pacientes de apellido Smith nacidos después de 1980
search-nl-hint = Tu texto y los parámetros de búsqueda de este servidor van al modelo de lenguaje. Los datos de pacientes nunca. La consulta que escribe se muestra abajo para que la revises y la ejecutes.
search-nl-working = Traduciendo…
search-nl-caveats = Ten en cuenta:
search-nl-unsupported = Eso no es una búsqueda que este servidor pueda ejecutar. Prueba describiendo los registros que quieres encontrar.

search-nl-example-1 = Pacientes mujeres mayores de 65 con diagnóstico de diabetes
search-nl-example-2 = Observaciones de los últimos 30 días, las más recientes primero
search-nl-example-3 = Encuentros en Boston General todavía en curso

search-setup-heading = La búsqueda en lenguaje natural está disponible
search-setup-body = Convierte descripciones en lenguaje llano en consultas de búsqueda FHIR. Necesita una clave de API de un modelo de lenguaje — el servidor la lee del entorno y nunca llega a esta página. Mientras no haya una, usa el constructor visual de abajo.
search-setup-key-placeholder = tu clave de API
search-setup-disable = Para eliminar la función por completo — endpoint, página y este aviso — define HFS_NL_SEARCH_ENABLED=false.
search-setup-docs = Leer el instructivo

## Editor de recursos (#264)

editor-heading = Editor de recursos
editor-lede = Edita un recurso contra su esquema: añade cualquier elemento que el esquema permita, a cualquier profundidad — incluidas extensiones, en cualquier nodo que las acepte.
editor-title = Editar recurso
editor-view-label = Cómo editar
editor-view-form = Formulario guiado
editor-view-json = JSON
editor-save = Guardar cambios
editor-delete = Eliminar
editor-remove = Quitar este nodo
editor-saved = Guardado.
editor-load-error = No se pudo cargar ese recurso.
editor-confirm-delete = ¿Eliminar este recurso? No se puede deshacer.
editor-invalid-json = Eso no es JSON válido, así que no puede editarse como formulario. Tu texto queda intacto.
editor-source-hint = Edita el código directamente. Al volver al formulario guiado se interpreta.

editor-add = Añadir elemento
editor-must-support-badge = MS
editor-binding-hint = Ligado a un value set — los códigos salen de él; se muestra la fuerza
editor-legend-live = Se comprueba al escribir: estructura, cardinalidad, bindings requeridos
editor-legend-save = Se comprueba al guardar: constraints y terminología
editor-deferred-badge = al guardar
editor-deferred-hint = Los códigos se verifican contra el value set al guardar (y en vivo en el picker si hay servidor de terminología configurado)
editor-must-support-hint = Must-support: se espera que los consumidores de este perfil manejen este elemento
editor-add-filter = Filtrar elementos
editor-add-another = añadir otro
editor-pick-type = Elige un tipo…
editor-extension-url = URL de la extensión
editor-add-extension = Añadir extensión

editor-valid = Sin problemas.
editor-issues = { $count ->
    [one] { $count } problema
   *[other] { $count } problemas
}

editor-modifier-badge = modificadora
editor-modifier-warning = Una extensión modificadora cambia el significado del recurso. Un sistema que no la reconozca debe negarse a procesarlo.
editor-unknown-badge = fuera del esquema
editor-unknown-hint = El esquema no describe este elemento. Se muestra para que no se pierda en silencio, y se conserva al guardar.

editor-primitive-extension-badge = + extensión
editor-primitive-extension-hint = Este valor lleva extensiones propias (un hermano `_` en el JSON). Se conservan al guardar.

editor-collapse-all = Colapsar todo
editor-expand-all = Expandir todo
editor-edit-raw = Editar crudo
editor-versions = Versiones
editor-versions-none = Sin versiones anteriores.
## Historial y versiones (#236)

## Espacio de recursos (#282)

resources-heading = Recursos
resources-lede = Explora, busca, crea y edita recursos FHIR. Busca en lenguaje natural o arma la consulta a mano, y abre cualquier resultado para editarlo.
resources-create-typed = Crear { $type }
resources-save-blocked = Corrige los problemas de validación antes de guardar.
resources-save-invalid = El JSON no es válido — corrígelo antes de guardar.
resources-edit-title = Editar recurso
resources-tab-edit = Editar
resources-tab-history = Historial
resources-types-heading = Tipos de recurso
rail-all-types-heading = Todos los tipos

queries-saved-group = Guardadas

nav-collapse = Colapsar menú

batch-heading = Batch / Transaction
batch-lede = Sube un Bundle FHIR, revisa las acciones que va a ejecutar, ejecútalo contra este servidor y lee el resultado de cada entrada.
batch-upload = Subir
batch-drop-hint = Suelta aquí un fichero JSON de bundle
batch-drop-browse = o haz clic para explorar
batch-invalid-json = Ese fichero no es JSON válido
batch-not-a-bundle = Ese JSON no es un Bundle FHIR
batch-bad-type = Aquí solo se ejecutan Bundles de tipo batch o transaction
batch-request = Petición
batch-entries = entradas
batch-semantics-batch = Batch: las entradas se ejecutan de forma independiente — una entrada fallida no detiene ni deshace las demás.
batch-semantics-transaction = Transaction: todo o nada — si alguna entrada falla, el servidor revierte el bundle completo.
batch-tab-actions = Acciones
batch-tab-json = JSON del bundle
batch-no-body = (sin cuerpo — esta entrada solo direcciona un recurso)
batch-cancel = Cancelar
batch-upload-another = Subir otro
batch-execute = Ejecutar
batch-response-heading = Resultados por acción
batch-sum-created = creados
batch-sum-updated = actualizados
batch-sum-other = lecturas/otros
batch-sum-failed = fallidos
batch-request-failed = La petición falló
batch-back = Volver al bundle
batch-execute-again = Ejecutar de nuevo

## Bulk Import workspace (#527)

bulk-import-title = Importación masiva
bulk-import-new = Nueva submission
bulk-import-create-title = Crear Bulk Submission
bulk-import-field-name = Nombre de la submission
bulk-import-field-recipient = URL base del receptor
bulk-import-field-recipient-hint = La URL base del servidor donde se enviarán los datos.
bulk-import-auth = Autenticación
bulk-import-auth-hint = Cómo autenticarse ante el servidor receptor.
bulk-import-auth-none = Ninguna
bulk-import-auth-none-hint = No se enviará cabecera de autorización.
bulk-import-auth-backend = Autenticación backend services
bulk-import-auth-backend-hint = Obtiene un token de acceso y lo envía como Bearer en la cabecera de autorización.
bulk-import-field-client-id = Client ID
bulk-import-field-client-id-hint = Registre este proveedor de datos con el receptor y obtenga un client ID.
bulk-import-field-token-url = URL del token
bulk-import-field-token-url-hint = URL del endpoint de token del servidor de autorización.
bulk-import-jwks-hint = Registre la clave pública de este servidor con el destinatario mediante la URL de JWKS:
bulk-import-test-auth = Probar autenticación
bulk-import-test-auth-ok = Autenticación correcta.
bulk-import-create-submit = Crear submission
bulk-import-unavailable = El backend de almacenamiento no aloja el settings store; no se pueden guardar submissions.
bulk-import-submissions = Submissions
bulk-import-records = registros
bulk-import-col-name = Nombre
bulk-import-col-status = Estado
bulk-import-col-created = Creada
bulk-import-col-manifests = Manifests
bulk-import-col-destination = Destino
bulk-import-empty = Aún no hay submissions. Cree una para empezar.
bulk-import-all = Todas las submissions
bulk-import-status-not-started = Sin iniciar
bulk-import-status-in-progress = En curso
bulk-import-status-stopped = Detenida
bulk-import-status-completed = Completada
bulk-import-detail-recipient = Receptor de datos
bulk-import-detail-id = ID de submission
bulk-import-detail-submitter = Remitente
bulk-import-detail-created = Creada
bulk-import-detail-status = Estado
bulk-import-detail-auth = Autenticación
bulk-import-abort = Abortar
bulk-import-complete = Completar
bulk-import-delete = Eliminar
bulk-import-add-manifest = Añadir manifest
bulk-import-add-manifest-title = Añadir manifest
bulk-import-add-manifest-submit = Añadir
bulk-import-field-manifest-url = URL del manifest
bulk-import-field-manifest-url-hint = URL de un Bulk Export Manifest con un conjunto de datos FHIR precoordinado.
bulk-import-field-fhir-base = URL base FHIR
bulk-import-field-fhir-base-hint = URL base que usará el receptor al resolver referencias relativas. Déjela vacía para usar la URL base del manifest.
bulk-import-field-output-format = Formato de salida
bulk-import-field-output-format-hint = El formato de los archivos Bulk Data del manifest.
bulk-import-field-headers = Cabeceras de petición de archivos
bulk-import-field-headers-hint = Cabeceras HTTP que el receptor debe usar al pedir un archivo de datos, una "Nombre: valor" por línea.
bulk-import-manifests = Manifests
bulk-import-no-manifests = Aún no hay manifests. Añada uno para enviar datos.
bulk-import-submit = Enviar
bulk-import-submit-all = Enviar todo
bulk-import-remove = Quitar
bulk-import-log = Registro de la submission
bulk-import-log-empty = Todavía no se ha enviado nada.
bulk-import-field-submitter-system = Sistema del remitente
bulk-import-field-submitter-value = Valor del remitente
bulk-import-field-submitter-hint = Debe coincidir con un identificador registrado con el receptor (coordinado fuera de banda). Déjelo vacío para usar los valores generados.
bulk-import-field-submission-id = ID de submission
bulk-import-field-submission-id-hint = Único por remitente. Déjelo vacío para generar un UUID.
bulk-import-processing = Procesando
bulk-import-processing-waiting = Esperando el primer reporte de estado del receptor…
bulk-import-result = Resultado
bulk-import-result-finished = Procesamiento terminado a las
bulk-import-result-outputs = Archivos de salida
bulk-import-result-errors = Archivos de error
bulk-import-abort-manifest = Abortar
ui-cancel = Cancelar
ui-close = Cerrar
editor-orphans-title = Estos problemas aún no tienen campo — añada los elementos para corregirlos
editor-hint-date = FHIR date: YYYY, YYYY-MM o YYYY-MM-DD
editor-hint-datetime = FHIR dateTime: YYYY, YYYY-MM, YYYY-MM-DD o un timestamp completo con zona horaria (2024-05-17T14:30:00+02:00)
editor-hint-time = FHIR time: HH:MM:SS
editor-hint-instant = FHIR instant: timestamp completo con zona horaria, p. ej. 2024-05-17T14:30:00.000Z

## Página de suscripciones (#580)

subs-title = Suscripciones
subs-lede = Vista de solo lectura del motor de suscripciones: cada suscripción registrada, su canal, estado en vivo y contadores de entrega.
subs-unavailable = El motor de suscripciones no está habilitado en este servidor.
subs-empty = No hay suscripciones registradas para este tenant.
subs-card-failing = Fallando
subs-card-failing-sub = requiere atención
subs-card-idle = Inactivas
subs-card-idle-sub = sin clientes
subs-card-active = Activas
subs-card-active-sub = entregando
subs-card-delivered = Entregadas en 24 h
subs-card-delivered-sub = { $rate }% al primer intento
subs-card-delivered-none = sin entregas en la ventana
subs-table-heading = Suscripciones
subs-sort = Ordenar
subs-sort-status = Estado
subs-sort-sent = Más enviadas
subs-sort-fails = Racha de fallos
subs-col-subscription = Suscripción
subs-col-channel = Canal
subs-col-status = Estado
subs-col-last24 = Últimas 24 h
subs-col-sent = Enviadas
subs-col-fails = Racha de fallos
subs-state-active = activa
subs-state-error = error
subs-state-idle = 0 clientes
subs-state-requested = solicitada
subs-state-off = apagada

## Bulk Export workspace (#537)

bulk-export-title = Exportación masiva
bulk-export-active-title = Exportaciones activas
bulk-export-active-link = Exportaciones activas
bulk-export-new = Nueva exportación
bulk-export-unavailable = El backend de almacenamiento no aloja el settings store; no se pueden rastrear los trabajos de exportación.
bulk-export-scope = ¿Qué desea exportar?
bulk-export-scope-system = Todo
bulk-export-scope-system-hint = El servidor completo — cada tipo de recurso que seleccione abajo.
bulk-export-scope-patient = Pacientes
bulk-export-scope-patient-hint = Cada paciente y los registros que le pertenecen. Nada ajeno a pacientes.
bulk-export-scope-group = Grupo
bulk-export-scope-group-hint = Solo los miembros de una cohorte ya definida.
bulk-export-field-group-id = ID del grupo
bulk-export-field-group-id-hint = Requerido para el alcance Grupo: el id del Group FHIR a exportar.
bulk-export-field-name = Nombre
bulk-export-field-name-placeholder = Registro de diabetes 2024
bulk-export-types = Tipos de recurso
bulk-export-types-hint = Deje todo sin marcar para exportar todos los tipos.
bulk-export-narrow = Acotar
bulk-export-field-elements = Elementos FHIR
bulk-export-field-type-filter = Filtro por tipo
bulk-export-field-since = Desde
bulk-export-since-all = Todo el tiempo
bulk-export-since-day = Último día
bulk-export-since-week = Últimos 7 días
bulk-export-since-month = Últimas 4 semanas
bulk-export-since-custom = Personalizado
bulk-export-field-since-custom = Instante personalizado
bulk-export-field-since-custom-hint = Se usa cuando Desde es Personalizado. RFC 3339, p. ej. 2026-08-01T00:00:00Z.
bulk-export-start = Iniciar exportación
bulk-export-running = en curso
bulk-export-none = Aún no hay exportaciones. Inicie una desde la página de Exportación masiva.
bulk-export-status-in-progress = En curso
bulk-export-status-complete = Completada
bulk-export-status-failed = Fallida
bulk-export-status-cancelled = Cancelada
bulk-export-progress = Progreso
bulk-export-progress-waiting = Esperando el primer reporte de estado…
bulk-export-files = Archivos
bulk-export-finished-in = terminada en
bulk-export-error = Error
bulk-export-cancel = Cancelar
bulk-export-retry = Reintentar

# Página CapabilityStatement (#653)
cap-title = Declaración de capacidades
cap-lede = Lo que este servidor hace ahora mismo, para el tenant y la versión FHIR seleccionados — compuesto en vivo desde /metadata.
cap-summary-heading = Resumen del servidor
cap-summary-description = Descripción
cap-summary-url = URL base
cap-summary-fhir-version = Versión FHIR
cap-summary-status = Estado
cap-summary-kind = Tipo
cap-summary-date = Fecha
cap-summary-formats = Formatos
cap-interactions-heading = Interacciones de sistema
cap-transaction-note = transaction se anuncia porque el backend activo soporta transacciones atómicas; batch está siempre disponible.
cap-operations-heading = Operaciones
cap-col-operation = Operación
cap-col-definition = Definición
cap-resources-heading = Capacidades por recurso
cap-filter-placeholder = Filtrar tipos…
cap-col-type = Tipo
cap-col-interactions = Interacciones
cap-col-search-params = Parámetros de búsqueda
cap-col-includes = Includes
cap-col-revincludes = Revincludes
cap-resources-empty = Ningún tipo de recurso coincide con el filtro.
cap-raw-toggle = CapabilityStatement en bruto (JSON)
cap-unavailable = No se pudo obtener la CapabilityStatement del servidor — la autollamada puede necesitar un token saliente cuando la autenticación está activada.
