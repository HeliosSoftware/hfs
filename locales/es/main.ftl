# Servidor FHIR Helios â€” catÃ¡logo de mensajes de la interfaz
# ConfiguraciÃ³n regional: EspaÃ±ol (es)
#
# Mantenga las mismas claves que en `en/main.ftl` (la configuraciÃ³n regional
# de origen). Las claves que falten recurren a inglÃ©s segÃºn la cadena de
# reserva descrita en docs/multi-language.md.

## Marca / tÃ©rminos compartidos

-app-name = Servidor FHIR Helios
-org-name = Helios Software

## Estructura de pÃ¡gina

app-title = { -app-name }
app-tagline = Un servidor FHIR rÃ¡pido y multiversiÃ³n

nav-dashboard = Panel
nav-terminology = TerminologÃ­a
nav-resources = Recursos
nav-settings = ConfiguraciÃ³n
nav-signout = Cerrar sesiÃ³n

## Selector de idioma

language-label = Idioma
language-en = InglÃ©s
language-es = EspaÃ±ol
language-de = AlemÃ¡n

## PÃ¡gina de inicio

home-lede = Interfaz renderizada en el servidor y basada en HTMX. Este panel se actualiza como un fragmento HTML.

## Panel de estado

status-last-checked = Ãšltima comprobaciÃ³n: { $timestamp }

## Panel / estado

dashboard-heading = Panel del servidor
health-status-ok = Todos los sistemas operativos
health-status-degraded = Algunos sistemas estÃ¡n degradados
health-uptime = Tiempo activo: { $duration }

resource-count = { $count ->
    [one] { $count } recurso
   *[other] { $count } recursos
}

## ExploraciÃ³n de terminologÃ­a

terminology-search-label = Buscar CodeSystems y ValueSets
terminology-search-placeholder = p. ej. 73211009, Â«diabetesÂ», http://snomed.info/sct
terminology-display-language = Idioma de visualizaciÃ³n
terminology-no-results = No se encontraron conceptos coincidentes.

## Acciones comunes

action-search = Buscar
action-save = Guardar
action-cancel = Cancelar
action-retry = Reintentar

## Errores (refleja el texto de OperationOutcome; vÃ©ase docs/multi-language.md Â§5)

error-not-found = No se encontrÃ³ el recurso solicitado.
error-unauthorized = No estÃ¡ autorizado para realizar esta acciÃ³n.
error-generic = Algo saliÃ³ mal. Vuelva a intentarlo.

## Estructura del panel (Figma Â«Dashboard V1.1Â»)

nav-section-work = Trabajo
nav-section-batch-data = Lotes y datos
nav-section-server = Servidor
nav-section-conditional = Condicional

nav-home = Inicio
nav-search = Buscar
nav-resource-editor = Editor de recursos
nav-history-versions = Historial y versiones
nav-compartments = Compartimentos
nav-batch-transaction = Lote / TransacciÃ³n
nav-import = Importar
nav-export = Exportar
nav-sql-on-fhir = SQL-on-FHIR
nav-capability-conformance = Capacidad y conformidad
nav-search-parameters = ParÃ¡metros de bÃºsqueda
nav-admin-ops = AdministraciÃ³n / Operaciones
nav-subscriptions = Suscripciones
nav-tenants = Tenants

## Mantenimiento de tenants (/ui/tenants)

tenants-title = Mantenimiento de tenants
tenants-unavailable = El registro de tenants no estÃ¡ disponible en este backend de almacenamiento.
tenants-stat-total = Tenants totales
tenants-stat-total-sub = { $count ->
    [one] { $count } registrado
   *[other] { $count } registrados
}
tenants-stat-resources = Recursos almacenados
tenants-stat-resources-sub = en todos los tenants
tenants-search-placeholder = Buscar por nombre o id de tenantâ€¦
tenants-add = AÃ±adir tenant
tenants-add-title = AÃ±adir un tenant
tenants-field-id = Id del tenant
tenants-field-id-hint = Se usa en la API (cabecera X-Tenant-ID, prefijo de URL, claim del JWT).
tenants-field-name = Nombre visible (opcional)
tenants-field-name-hint = Una etiqueta legible; no se usa para el enrutamiento.
tenants-add-submit = Aprovisionar tenant
tenants-col-tenant = Tenant
tenants-col-resources = Recursos
tenants-col-created = Creado
tenants-col-actions = Acciones
tenants-empty = NingÃºn tenant coincide.
tenants-unregistered = sin registrar
tenants-delete = Eliminar tenant
tenants-delete-confirm = Â¿Dar de baja el tenant Â«{ $id }Â»? Sus datos se conservan salvo que se purguen vÃ­a API.

tenant-heading = Tenants
tenant-all = Todos los tenants
tenant-search-placeholder = Buscar tenants

theme-label = Tema
theme-light = Tema claro
theme-dark = Tema oscuro

fhir-version = FHIR { $version }
fhir-version-heading = VersiÃ³n FHIR

card-resource-types = Tipos de recursos
card-resource-types-sub = habilitados para { $version }
card-stored-resources = Recursos almacenados
card-stored-resources-sub = en el tenant activo
card-export-jobs = Trabajos de exportaciÃ³n
card-export-jobs-sub = en ejecuciÃ³n ({ $queued } en cola)
card-uptime = Disponibilidad
card-uptime-sub = Ãºltimos 30 dÃ­as

chart-title = Recursos FHIR en el tiempo
chart-expand = Ampliar el grÃ¡fico
chart-window = Intervalo de tiempo del grÃ¡fico

## Pie de pÃ¡gina

footer-copyright = Â© { $year } { -org-name }

## Historial y versiones (#236)

history-heading = Historial y versiones
history-lede = Compara dos versiones de un recurso. El almacenamiento estÃ¡ totalmente versionado; esto lo lee con la API estÃ¡ndar _history y vread.
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
history-not-found = No hay historial para ese recurso â€” revisa el tipo y el id.
history-diff-heading = { $from }
history-metadata-hidden = { $count ->
    [one] { $count } cambio de metadatos oculto
   *[other] { $count } cambios de metadatos ocultos
}
history-textual = Ver diff de texto completo
history-only-metadata = Entre estas versiones solo cambiaron los metadatos.
history-identical = Estas dos versiones son idÃ©nticas.
history-deleted = { $version } es una eliminaciÃ³n â€” no hay contra quÃ© comparar.
history-parse-error = No se pudieron leer esas versiones como JSON.
## Saved queries (#234)

nav-saved-queries = Consultas guardadas

queries-heading = Consultas guardadas
queries-lede = Guarda consultas de bÃºsqueda FHIR por tipo de recurso, ordenadas por su Ãºltima ejecuciÃ³n. Se guardan en tu configuraciÃ³n de usuario y te siguen entre dispositivos.
queries-add-heading = Guardar una consulta
queries-type-label = Tipo de recurso
queries-type-placeholder = p. ej. Patient
queries-name-label = Nombre
queries-name-placeholder = p. ej. Smith en Boston
queries-query-label = Cadena de consulta
queries-query-placeholder = p. ej. name=smith&address-city=Boston
queries-empty = AÃºn no hay consultas guardadas. Guarda una arriba para empezar.
queries-never-run = Nunca ejecutada
queries-run = Ejecutar
queries-rename = Renombrar
queries-delete = Eliminar
queries-rename-prompt = Nuevo nombre
queries-confirm-delete = Â¿Eliminar Â«{ $name }Â»?
queries-unavailable = Las consultas guardadas no estÃ¡n disponibles: el backend de almacenamiento de este servidor no admite configuraciÃ³n por usuario.

## Visor de SearchParameters (#238)

sp-heading = ParÃ¡metros de bÃºsqueda
sp-lede = Explora los parÃ¡metros con los que este servidor resuelve las bÃºsquedas, filtrados por tipo de recurso base. Los parÃ¡metros almacenados se pueden crear, editar y eliminar; el registro recoge los cambios por tenant.
sp-version-label = VersiÃ³n FHIR
sp-spec-missing = No se encontrÃ³ el bundle completo de la especificaciÃ³n (search-parameters-*.json) en el directorio de datos â€” solo se muestran los parÃ¡metros mÃ­nimos embebidos.
sp-rail-label = Filtro de recursos
sp-rail-search = Filtrar tipos
sp-rail-recent = Usados recientemente
sp-rail-types = Tipos de recurso
sp-rail-all = Todos los tipos
sp-facet-type = Tipo
sp-facet-type-label = Filtrar por tipo de parÃ¡metro
sp-facet-source = Origen
sp-facet-source-label = Filtrar por origen
sp-source-embedded = embebido
sp-source-stored = almacenado
sp-source-config = configuraciÃ³n
sp-chip-conflict = conflicto
sp-chip-overrides = anula la spec
sp-chip-shadowed = eclipsado
sp-col-code = CÃ³digo
sp-col-type = Tipo
sp-col-base = Base
sp-col-expression = ExpresiÃ³n
sp-col-source = Origen
sp-total = { $count } parÃ¡metros
sp-pagination-label = PÃ¡ginas
sp-page-prev = Anterior
sp-page-next = Siguiente
sp-detail-label = Detalle del parÃ¡metro
sp-detail-empty = NingÃºn parÃ¡metro seleccionado
sp-detail-empty-hint = Selecciona una fila para inspeccionar su definiciÃ³n, su expresiÃ³n y cÃ³mo se resuelve en el registro.
sp-detail-readonly = ParÃ¡metro de la especificaciÃ³n (compilado desde el archivo de datos) â€” solo lectura.
sp-field-url = URL canÃ³nica
sp-field-name = Nombre
sp-field-status = Estado
sp-field-base = Tipos de recurso base
sp-field-expression = ExpresiÃ³n FHIRPath
sp-field-description = DescripciÃ³n
sp-field-target = Tipos destino
sp-field-components = Componentes
sp-status-hint = El cargador promueve el estado draft de la especificaciÃ³n a active al cargar.
sp-note-conflict = (base, code) duplicado dentro del mismo origen que { $url } â€” el registro rechaza esta colisiÃ³n (DuplicateCode).
sp-note-overrides = Anula a { $url } en (base, code): una definiciÃ³n almacenada tiene precedencia sobre el parÃ¡metro de la spec, asÃ­ que esta resuelve las bÃºsquedas. El registro emite un WARN con ambas URLs.
sp-note-shadowed = Eclipsado por { $url } en (base, code): un origen de mayor precedencia resuelve las bÃºsquedas de este slot.
sp-note-empty-expression = ExpresiÃ³n vacÃ­a: el extractor no indexa ninguna fila, asÃ­ que toda bÃºsqueda con este parÃ¡metro devuelve vacÃ­o en silencio.
sp-note-no-target = ParÃ¡metro de referencia sin tipos destino: la bÃºsqueda encadenada no puede resolver el tipo referenciado.
sp-note-choice-type = ExpresiÃ³n de tipo choice: el extractor reescribe ofType(T) / as T al elemento concreto (por ejemplo valueQuantity) antes de evaluar contra el JSON almacenado.
sp-new = Nuevo parÃ¡metro de bÃºsqueda
sp-edit = Editar
sp-delete = Eliminar
sp-delete-confirm = Â¿Eliminar este parÃ¡metro de bÃºsqueda almacenado? Las bÃºsquedas que lo usan dejarÃ¡n de coincidir cuando el registro se actualice.
cmp-new = Nueva definiciÃ³n de compartimento
cmp-edit = Editar
cmp-delete = Eliminar
cmp-delete-confirm = Â¿Eliminar esta definiciÃ³n de compartimento? Sus rutas de compartimento dejarÃ¡n de resolverse.
crud-delete-failed = Error al eliminar

## Visor y probador de compartments (#237)

cmp-heading = Compartimentos
cmp-lede = Las definiciones de compartment con las que este servidor enruta las peticiones /{"{"}compartment{"}"}/{"{"}id{"}"}/{"{"}type{"}"}, y un probador que responde: Â¿estÃ¡ este tipo en este compartment, mediante quÃ© parÃ¡metros, y quÃ© bÃºsqueda ejecuta el servidor?
cmp-rail-label = Definiciones de compartment
cmp-rail-heading = Compartimentos
cmp-degraded = Las definiciones de compartimento no se pudieron cargar de este servidor en este momento â€” la auto-llamada a /CompartmentDefinition fallÃ³ (con autenticaciÃ³n habilitada esto suele significar que el token de servicio saliente falta o es invÃ¡lido). La pÃ¡gina reintenta en la siguiente peticiÃ³n.
cmp-rail-note = Las definiciones son recursos almacenados, sembrados desde la especificaciÃ³n FHIR al arrancar. Las ediciones y eliminaciones aquÃ­ son por tenant.
cmp-tabs-label = Secciones del compartment
cmp-tab-definition = DefiniciÃ³n
cmp-tab-members = Miembros
cmp-tab-tester = Probador
cmp-field-code = CÃ³digo
cmp-field-status = Estado
cmp-field-url = URL canÃ³nica
cmp-field-version = VersiÃ³n
cmp-field-publisher = Editor
cmp-field-description = DescripciÃ³n
cmp-field-search = search
cmp-field-experimental = experimental
cmp-search-why = Apagado significarÃ­a que ninguna ruta de compartment resuelve para este compartment.
cmp-on = activado
cmp-off = desactivado
cmp-yes = sÃ­
cmp-no = no
cmp-readonly-note = Solo lectura: estos valores provienen de las definiciones de la especificaciÃ³n compiladas en el servidor.
cmp-filter-members = Miembros
cmp-filter-all = Todos los tipos
cmp-filter-excluded = Excluidos
cmp-member = miembro
cmp-excluded = excluido
cmp-tester-id = Id
cmp-tester-target = Tipo destino (o *)
cmp-tester-run = Probar
cmp-result-member = âœ“ miembro â€” vÃ­a { $params }
cmp-result-flat = // bÃºsqueda plana equivalente
cmp-result-member-note = El servidor resuelve la ruta de compartment a esta bÃºsqueda sobre los parÃ¡metros de referencia del tipo.
cmp-result-self = âœ“ miembro â€” el propio recurso del compartment ({"{"}def{"}"})
cmp-result-self-note = La instancia del compartment estÃ¡ trivialmente en su propio compartment; la ruta lee el recurso directamente.
cmp-result-notmember = âœ• { $type } no es miembro de este compartment
cmp-result-notmember-note = El servidor devuelve 404 con un OperationOutcome para tipos que no son miembros del compartment.
cmp-result-fanout = Se expande a { $count } tipos miembro
cmp-result-fanout-note = Los tipos excluidos se omiten, no fallan â€” el fan-out descarta los tipos no miembro en lugar de dar error.
queries-builder-heading = Constructor de bÃºsquedas
queries-url-label = URL de bÃºsqueda FHIR
queries-url-placeholder = GET /Patient?name=smith&birthdate=ge1980-01-01
queries-builder-hint = Edita la URL GET directamente o mediante las filas de abajo â€” se mantienen sincronizadas. Ejecutar corre la bÃºsqueda aquÃ­ mismo y la registra en Recientes; ponle un nombre para conservarla en la lista.
queries-recent = Recientes
queries-recent-heading = BÃºsquedas recientes
queries-recent-empty = AÃºn no hay bÃºsquedas recientes â€” ejecuta una para registrarla aquÃ­.
queries-invalid-url = Escribe una bÃºsqueda como GET /Patient?name=smith â€” el tipo de recurso sale de la ruta.

queries-conditions = Condiciones
queries-add-condition = AÃ±adir condiciÃ³n
queries-includes = Includes
queries-result-controls = Controles de resultado
queries-remove = Quitar
queries-match-is = es
queries-or = + o
plain-pill = En lenguaje claro
plain-find = Buscar registros de {"{type}"}
plain-clause = {"{path}"} {"{verb}"} {"{value}"}
plain-and = y
plain-or = o
plain-arrow = {" "}â†’
plain-has = que tienen un {"{type}"} relacionado cuyo {"{param}"} {"{verb}"} {"{value}"}
plain-include = Devolviendo tambiÃ©n el {"{param}"} de cada {"{type}"}{"{target}"}
plain-revinclude = MÃ¡s cada {"{type}"} cuyo {"{param}"} apunta aquÃ­
plain-iterate = (repetidamente)
plain-count = Mostrando {"{n}"} por pÃ¡gina
plain-sort = Ordenado por {"{sort}"}
plain-verb-is = es
plain-verb-contains = contiene
plain-verb-exact = es exactamente
plain-verb-missing = estÃ¡ presente/ausente
plain-verb-not = no es
plain-verb-text = coincide con el texto
plain-verb-in = estÃ¡ en el value set
plain-verb-not-in = no estÃ¡ en el value set
plain-verb-identifier = tiene el identificador
plain-verb-of-type = tiene un identificador de tipo
plain-verb-ge = es igual o posterior a
plain-verb-le = es igual o anterior a
plain-verb-gt = es posterior a
plain-verb-lt = es anterior a
plain-verb-ne = no es
plain-verb-eq = es
plain-verb-sa = comienza despuÃ©s de
plain-verb-eb = termina antes de
plain-verb-ap = es aproximadamente
queries-related-heading = Incluir datos relacionados
queries-related-sub = AÃ±ade recursos conectados a los resultados.
queries-related-add-include = Incluir un recurso al que apunta
queries-related-add-revinclude = Incluir recursos que apuntan aquÃ­
queries-iterate = Iterar
queries-sort-label = Orden
queries-sort-default = Predeterminado
queries-sort-recent = MÃ¡s recientes
queries-sort-oldest = MÃ¡s antiguos
queries-sort-id = ID
queries-modify-heading = Modificadores
queries-mod-exact = valor completo, incl. mayÃºsculas y acentos
queries-mod-contains = coincide en cualquier parte del texto
queries-mod-missing = el campo estÃ¡ presente / ausente
queries-mod-text = tratamiento avanzado de texto
queries-mod-not = ningÃºn valor coincide
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
queries-has-via = enlazado vÃ­a
queries-has-where = donde su
queries-add-has = â§‰ Filtrar un recurso que enlaza aquÃ­
queries-param-placeholder = parÃ¡metro
queries-value-placeholder = valor
queries-results = Resultados
queries-results-total = { $count } resultados
queries-results-included = { $count } incluidos
queries-results-empty = Sin resultados.
queries-open-tab = Abrir en pestaÃ±a nueva
queries-col-updated = Actualizado
queries-prev = Anterior
queries-next = Siguiente

queries-rail-heading = Tipos de recurso
queries-rail-filter = Filtrar tipos

## BÃºsqueda â€” lenguaje natural y constructor visual (#255)

search-heading = Buscar
search-lede = Describe lo que buscas, o arma la consulta a mano. En ambos casos obtienes una bÃºsqueda FHIR que puedes leer, corregir y ejecutar.
search-query-tag = CONSULTA
search-copy = Copiar la consulta

search-mode-label = CÃ³mo escribir la consulta
search-mode-nl = Lenguaje natural
search-mode-builder = Constructor visual

search-nl-label = Describe la bÃºsqueda
search-nl-placeholder = Describe lo que buscas â€” p. ej. pacientes de apellido Smith nacidos despuÃ©s de 1980
search-nl-hint = Tu texto y los parÃ¡metros de bÃºsqueda de este servidor van al modelo de lenguaje. Los datos de pacientes nunca. La consulta que escribe se muestra abajo para que la revises y la ejecutes.
search-nl-working = Traduciendoâ€¦
search-nl-caveats = Ten en cuenta:
search-nl-unsupported = Eso no es una bÃºsqueda que este servidor pueda ejecutar. Prueba describiendo los registros que quieres encontrar.

search-nl-example-1 = Pacientes mujeres mayores de 65 con diagnÃ³stico de diabetes
search-nl-example-2 = Observaciones de los Ãºltimos 30 dÃ­as, las mÃ¡s recientes primero
search-nl-example-3 = Encuentros en Boston General todavÃ­a en curso

search-setup-heading = La bÃºsqueda en lenguaje natural estÃ¡ disponible
search-setup-body = Convierte descripciones en lenguaje llano en consultas de bÃºsqueda FHIR. Necesita una clave de API de un modelo de lenguaje â€” el servidor la lee del entorno y nunca llega a esta pÃ¡gina. Mientras no haya una, usa el constructor visual de abajo.
search-setup-key-placeholder = tu clave de API
search-setup-disable = Para eliminar la funciÃ³n por completo â€” endpoint, pÃ¡gina y este aviso â€” define HFS_NL_SEARCH_ENABLED=false.
search-setup-docs = Leer el instructivo

## Editor de recursos (#264)

editor-heading = Editor de recursos
editor-lede = Edita un recurso contra su esquema: aÃ±ade cualquier elemento que el esquema permita, a cualquier profundidad â€” incluidas extensiones, en cualquier nodo que las acepte.
editor-title = Editar recurso
editor-view-label = CÃ³mo editar
editor-view-form = Formulario guiado
editor-view-json = JSON
editor-save = Guardar cambios
editor-delete = Eliminar
editor-remove = Quitar este nodo
editor-saved = Guardado.
editor-load-error = No se pudo cargar ese recurso.
editor-confirm-delete = Â¿Eliminar este recurso? No se puede deshacer.
editor-invalid-json = Eso no es JSON vÃ¡lido, asÃ­ que no puede editarse como formulario. Tu texto queda intacto.
editor-source-hint = Edita el cÃ³digo directamente. Al volver al formulario guiado se interpreta.

editor-add = AÃ±adir elemento
editor-must-support-badge = MS
editor-binding-hint = Ligado a un value set â€” los cÃ³digos salen de Ã©l; se muestra la fuerza
editor-legend-live = Se comprueba al escribir: estructura, cardinalidad, bindings requeridos
editor-legend-save = Se comprueba al guardar: constraints y terminologÃ­a
editor-deferred-badge = al guardar
editor-deferred-hint = Los cÃ³digos se verifican contra el value set al guardar (y en vivo en el picker si hay servidor de terminologÃ­a configurado)
editor-must-support-hint = Must-support: se espera que los consumidores de este perfil manejen este elemento
editor-add-filter = Filtrar elementos
editor-add-another = aÃ±adir otro
editor-pick-type = Elige un tipoâ€¦
editor-extension-url = URL de la extensiÃ³n
editor-add-extension = AÃ±adir extensiÃ³n

editor-valid = Sin problemas.
editor-issues = { $count ->
    [one] { $count } problema
   *[other] { $count } problemas
}

editor-modifier-badge = modificadora
editor-modifier-warning = Una extensiÃ³n modificadora cambia el significado del recurso. Un sistema que no la reconozca debe negarse a procesarlo.
editor-unknown-badge = fuera del esquema
editor-unknown-hint = El esquema no describe este elemento. Se muestra para que no se pierda en silencio, y se conserva al guardar.

editor-primitive-extension-badge = + extensiÃ³n
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
resources-create = Crear nuevo
resources-save-blocked = Corrige los problemas de validaciÃ³n antes de guardar.
resources-save-invalid = El JSON no es vÃ¡lido â€” corrÃ­gelo antes de guardar.
resources-edit-title = Editar recurso
resources-tab-edit = Editar
resources-tab-history = Historial
resources-types-heading = Tipos de recurso

queries-saved-group = Guardadas

nav-collapse = Colapsar menÃº

batch-heading = Batch / Transaction
batch-lede = Sube un Bundle FHIR, revisa las acciones que va a ejecutar, ejecÃºtalo contra este servidor y lee el resultado de cada entrada.
batch-upload = Subir
batch-drop-hint = Suelta aquÃ­ un fichero JSON de bundle
batch-drop-browse = o haz clic para explorar
batch-invalid-json = Ese fichero no es JSON vÃ¡lido
batch-not-a-bundle = Ese JSON no es un Bundle FHIR
batch-bad-type = AquÃ­ solo se ejecutan Bundles de tipo batch o transaction
batch-request = PeticiÃ³n
batch-entries = entradas
batch-semantics-batch = Batch: las entradas se ejecutan de forma independiente â€” una entrada fallida no detiene ni deshace las demÃ¡s.
batch-semantics-transaction = Transaction: todo o nada â€” si alguna entrada falla, el servidor revierte el bundle completo.
batch-tab-actions = Acciones
batch-tab-json = JSON del bundle
batch-no-body = (sin cuerpo â€” esta entrada solo direcciona un recurso)
batch-cancel = Cancelar
batch-upload-another = Subir otro
batch-execute = Ejecutar
batch-response-heading = Resultados por acciÃ³n
batch-sum-created = creados
batch-sum-updated = actualizados
batch-sum-other = lecturas/otros
batch-sum-failed = fallidos
batch-request-failed = La peticiÃ³n fallÃ³
batch-back = Volver al bundle
batch-execute-again = Ejecutar de nuevo

## Bulk Import workspace (#527)

bulk-import-title = ImportaciÃ³n masiva
bulk-import-new = Nueva submission
bulk-import-create-title = Crear Bulk Submission
bulk-import-field-name = Nombre de la submission
bulk-import-field-recipient = URL base del receptor
bulk-import-field-recipient-hint = La URL base del servidor donde se enviarÃ¡n los datos.
bulk-import-auth = AutenticaciÃ³n
bulk-import-auth-hint = CÃ³mo autenticarse ante el servidor receptor.
bulk-import-auth-none = Ninguna
bulk-import-auth-none-hint = No se enviarÃ¡ cabecera de autorizaciÃ³n.
bulk-import-auth-backend = AutenticaciÃ³n backend services
bulk-import-auth-backend-hint = Obtiene un token de acceso y lo envÃ­a como Bearer en la cabecera de autorizaciÃ³n.
bulk-import-field-client-id = Client ID
bulk-import-field-client-id-hint = Registre este proveedor de datos con el receptor y obtenga un client ID.
bulk-import-field-token-url = URL del token
bulk-import-field-token-url-hint = URL del endpoint de token del servidor de autorizaciÃ³n.
bulk-import-jwks-hint = Registre la clave pÃºblica de este servidor con el destinatario mediante la URL de JWKS:
bulk-import-test-auth = Probar autenticaciÃ³n
bulk-import-test-auth-ok = AutenticaciÃ³n correcta.
bulk-import-create-submit = Crear submission
bulk-import-unavailable = El backend de almacenamiento no aloja el settings store; no se pueden guardar submissions.
bulk-import-submissions = Submissions
bulk-import-records = registros
bulk-import-col-name = Nombre
bulk-import-col-status = Estado
bulk-import-col-created = Creada
bulk-import-col-manifests = Manifests
bulk-import-col-destination = Destino
bulk-import-empty = AÃºn no hay submissions. Cree una para empezar.
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
bulk-import-detail-auth = AutenticaciÃ³n
bulk-import-abort = Abortar
bulk-import-complete = Completar
bulk-import-delete = Eliminar
bulk-import-add-manifest = AÃ±adir manifest
bulk-import-add-manifest-title = AÃ±adir manifest
bulk-import-add-manifest-submit = AÃ±adir
bulk-import-field-manifest-url = URL del manifest
bulk-import-field-manifest-url-hint = URL de un Bulk Export Manifest con un conjunto de datos FHIR precoordinado.
bulk-import-field-fhir-base = URL base FHIR
bulk-import-field-fhir-base-hint = URL base que usarÃ¡ el receptor al resolver referencias relativas. DÃ©jela vacÃ­a para usar la URL base del manifest.
bulk-import-field-output-format = Formato de salida
bulk-import-field-output-format-hint = El formato de los archivos Bulk Data del manifest.
bulk-import-field-headers = Cabeceras de peticiÃ³n de archivos
bulk-import-field-headers-hint = Cabeceras HTTP que el receptor debe usar al pedir un archivo de datos, una "Nombre: valor" por lÃ­nea.
bulk-import-manifests = Manifests
bulk-import-no-manifests = AÃºn no hay manifests. AÃ±ada uno para enviar datos.
bulk-import-submit = Enviar
bulk-import-submit-all = Enviar todo
bulk-import-remove = Quitar
bulk-import-log = Registro de la submission
bulk-import-log-empty = TodavÃ­a no se ha enviado nada.
bulk-import-field-submitter-system = Sistema del remitente
bulk-import-field-submitter-value = Valor del remitente
bulk-import-field-submitter-hint = Debe coincidir con un identificador registrado con el receptor (coordinado fuera de banda). DÃ©jelo vacÃ­o para usar los valores generados.
bulk-import-field-submission-id = ID de submission
bulk-import-field-submission-id-hint = Ãšnico por remitente. DÃ©jelo vacÃ­o para generar un UUID.
bulk-import-processing = Procesando
bulk-import-processing-waiting = Esperando el primer reporte de estado del receptorâ€¦
bulk-import-result = Resultado
bulk-import-result-finished = Procesamiento terminado a las
bulk-import-result-outputs = Archivos de salida
bulk-import-result-errors = Archivos de error
bulk-import-abort-manifest = Abortar

## UI administrativa de HTS (crates/hts-ui) â€” stubs de Phase 1
##
## El catÃ¡logo completo se completa en Phase 1.4 / Phase 2 segÃºn
## `edson/docs/hts-ui-design.md` Â§7. Estos stubs cubren el layout base, la nav
## lateral y el placeholder del dashboard de la Phase 1 blocker slice. Deben
## mantenerse en paridad con en/de/main.ftl.

-hts-app-name = Servidor de TerminologÃ­a Helios
hts-app-title = { -hts-app-name }

hts-nav-section-work = TerminologÃ­a
hts-nav-section-tools = Herramientas
hts-nav-section-server = Servidor
hts-nav-dashboard = Panel
hts-nav-code-systems = Sistemas de cÃ³digos
hts-nav-value-sets = Conjuntos de valores
hts-nav-concept-maps = Mapas de conceptos
hts-nav-operations = Operaciones
hts-nav-import = Importar
hts-nav-diagnostics = DiagnÃ³stico

hts-fhir-version-heading = VersiÃ³n FHIR
hts-fhir-version = FHIR { $version }

hts-dashboard-title = Panel
hts-dashboard-subtitle = Estado del servidor de terminologÃ­a, inventario del catÃ¡logo y acciones rÃ¡pidas.

## Filas del panel (encabezados ocultos visualmente para lectores de pantalla).

hts-dashboard-row-status = Estado del servidor
hts-dashboard-row-inventory = Inventario cargado
hts-dashboard-row-metrics = MÃ©tricas de trÃ¡fico
hts-dashboard-quick-links = Accesos rÃ¡pidos

## Tarjetas del panel.

hts-dashboard-tile-status = Estado
hts-dashboard-tile-backend = Backend
hts-dashboard-tile-uptime = Tiempo activo
hts-dashboard-tile-fhir-version = VersiÃ³n FHIR
hts-dashboard-tile-loaded-systems = Sistemas cargados
hts-dashboard-tile-loaded-systems-hint = De TerminologyCapabilities.codeSystem[]
hts-dashboard-tile-bundled-data = Datos empaquetados
hts-dashboard-tile-bundled-data-value = { $mib } MiB
hts-dashboard-tile-bundled-data-hint = Del contenido de HTS_BOOTSTRAP_DIR
hts-dashboard-tile-requests = Solicitudes
hts-dashboard-tile-avg-latency = Latencia promedio
hts-dashboard-tile-metrics-hint = De /metrics â€” Wave 2

## Valores del campo `status` de /health, con clave para traducciÃ³n.

hts-dashboard-status-ok = OK

## Banner degradado (contrato del design doc Â§7).

hts-degraded-title = El backend de terminologÃ­a no estÃ¡ totalmente disponible
hts-degraded-body = Algunas tarjetas se ocultan hasta que HTS vuelva a estar accesible. Los controles interactivos se deshabilitan en las pÃ¡ginas afectadas.
hts-degraded-reason-client-build = No se pudo construir el cliente HTTP hacia HTS.
hts-degraded-reason-upstream-down = No se puede alcanzar el servidor de terminologÃ­a.
hts-degraded-reason-upstream-timeout = El servidor de terminologÃ­a no respondiÃ³ a tiempo.
hts-degraded-reason-upstream-error = El servidor de terminologÃ­a devolviÃ³ un error.
hts-degraded-reason-upstream-shape = El servidor de terminologÃ­a devolviÃ³ una respuesta con forma inesperada.
hts-degraded-reason-bootstrapping = El servidor de terminologÃ­a todavÃ­a estÃ¡ cargando sus datos iniciales.
hts-degraded-reason-unknown = El servidor de terminologÃ­a no estÃ¡ disponible temporalmente.

## Chip de dialecto (topbar, displayLanguage / Accept-Language de sesiÃ³n â€” Â§7.1).

hts-dialect-label = Dialecto
hts-dialect-prefix = dialecto:
hts-dialect-heading = Dialecto de sesiÃ³n
hts-dialect-hint = Controla displayLanguage en expansiones y Accept-Language en lecturas. Los campos por operaciÃ³n en Operaciones prevalecen.

## Partial de OperationOutcome (compartido â€” Â§7 / Â§11).

hts-outcome-severity = Severidad: { $severity }
hts-outcome-request-id = Id de solicitud: { $id }
hts-outcome-code-not-found = El recurso solicitado no fue encontrado.
hts-outcome-code-invalid = La solicitud fue rechazada por invÃ¡lida.
hts-outcome-code-too-costly = La operaciÃ³n solicitada fue rechazada por ser demasiado costosa.
hts-outcome-code-unknown = El servidor devolviÃ³ una incidencia que la UI no reconoce.
hts-degraded-since = Desde { $timestamp }

## HTS Slice B â€” Navegador de CodeSystem + detalle con banco de trabajo integrado
## (design doc Â§7.2 + Â§7.3). Cada clave tiene su equivalente en en/de/main.ftl.

## PÃ­ldoras de estado de CodeSystem (usadas en el navegador y en la cabecera del detalle).

hts-cs-status-draft = borrador
hts-cs-status-active = activo
hts-cs-status-retired = retirado
hts-cs-status-unknown = desconocido

## PÃ¡gina del navegador de CodeSystem.

hts-cs-browser-title = Sistemas de cÃ³digos
hts-cs-browser-subtitle = Explora el catÃ¡logo de CodeSystems del servidor de terminologÃ­a y abre cualquier fila para inspeccionar sus metadatos y su banco de trabajo.
hts-cs-browser-filter-legend = Filtrar CodeSystems
hts-cs-browser-filter-url = URL canÃ³nica
hts-cs-browser-filter-version = VersiÃ³n
hts-cs-browser-filter-name = Nombre
hts-cs-browser-filter-title = TÃ­tulo
hts-cs-browser-filter-status = Estado
hts-cs-browser-filter-search = Buscar
hts-cs-browser-filter-reset = Restablecer
hts-cs-browser-empty = NingÃºn CodeSystem coincide con estos filtros.
hts-cs-browser-load-more = Cargar mÃ¡s
hts-cs-browser-showing-count = Mostrando { $count ->
    [one] { $count } CodeSystem
   *[other] { $count } CodeSystems
}
hts-cs-browser-table-caption = CodeSystems que coinciden con los filtros activos.
hts-cs-browser-column-url = URL
hts-cs-browser-column-version = VersiÃ³n
hts-cs-browser-column-title = TÃ­tulo
hts-cs-browser-column-status = Estado
hts-cs-browser-error-title = No se pudieron listar los CodeSystems

## PÃ¡gina de detalle del CodeSystem.

hts-cs-detail-title = { $name } Â· CodeSystem
hts-cs-detail-title-fallback = CodeSystem
hts-cs-detail-eyebrow = CodeSystem
hts-cs-detail-section-identity = Identidad
hts-cs-detail-section-content = Contenido
hts-cs-detail-content-mode = Modo de contenido
hts-cs-detail-count = Cantidad de conceptos
hts-cs-detail-publisher = Publicador
hts-cs-detail-jurisdiction = JurisdicciÃ³n
hts-cs-detail-supersedes = Reemplaza a
hts-cs-detail-superseded-by = Reemplazado por
hts-cs-detail-tabs-label = Secciones del banco de trabajo del CodeSystem
hts-cs-detail-tab-metadata = Metadatos
hts-cs-detail-tab-lookup = Consulta
hts-cs-detail-tab-validate = Validar
hts-cs-detail-tab-subsumes = SubsunciÃ³n
hts-cs-detail-workbench-hint = Elige una operaciÃ³n para ejecutarla sobre este CodeSystem.
hts-cs-detail-result-empty = Ejecuta la operaciÃ³n para ver su resultado aquÃ­.

## Formulario y resultados de $lookup.

hts-cs-lookup-heading = Consultar un concepto
hts-cs-lookup-code = CÃ³digo
hts-cs-lookup-version = VersiÃ³n
hts-cs-lookup-display-language = Idioma de visualizaciÃ³n
hts-cs-lookup-display-language-placeholder = p. ej. es-ES
hts-cs-lookup-properties-legend = Propiedades
hts-cs-lookup-designations = Designaciones
hts-cs-lookup-properties = Propiedades
hts-cs-lookup-no-match = HTS no devolviÃ³ ningÃºn concepto coincidente.

## Formulario y resultados de $validate-code.

hts-cs-validate-heading = Validar un cÃ³digo
hts-cs-validate-mode-legend = Modo de entrada
hts-cs-validate-mode-code = CÃ³digo simple
hts-cs-validate-mode-coding = Coding
hts-cs-validate-code = CÃ³digo
hts-cs-validate-display = VisualizaciÃ³n
hts-cs-validate-coding-legend = Coding
hts-cs-validate-coding-system = sistema
hts-cs-validate-coding-code = cÃ³digo
hts-cs-validate-coding-display = visualizaciÃ³n
hts-cs-validate-badge-true = vÃ¡lido
hts-cs-validate-badge-false = invÃ¡lido
hts-cs-validate-message = Mensaje

## Formulario y resultados de $subsumes.

hts-cs-subsumes-heading = Probar subsunciÃ³n
hts-cs-subsumes-scoped-system = Sistema (fijo)
hts-cs-subsumes-code-a = CÃ³digo A
hts-cs-subsumes-code-b = CÃ³digo B
hts-cs-subsumes-outcome-equivalent = Los cÃ³digos son equivalentes.
hts-cs-subsumes-outcome-subsumes = El cÃ³digo A subsume al cÃ³digo B.
hts-cs-subsumes-outcome-subsumed-by = El cÃ³digo A estÃ¡ subsumido por el cÃ³digo B.
hts-cs-subsumes-outcome-not-subsumed = Ninguno subsume al otro.

## Cromo compartido del banco de trabajo (reutilizado por Slice C/D/E).

hts-workbench-run = Ejecutar
hts-workbench-raw-response = Solicitud y respuesta sin procesar
hts-workbench-copy-url = URL de la solicitud
hts-workbench-format-json = JSON
hts-workbench-format-xml = XML

## RazÃ³n degradada adicional para 404 al leer CS (states matrix Â§7.3).

hts-degraded-reason-upstream-not-found = El servidor de terminologÃ­a no encontrÃ³ ese recurso.

## HTS Slice C â€” Navegador de ValueSet + detalle con banco de trabajo $expand
## (design doc Â§7.4 + Â§7.4.1). Cada clave aquÃ­ tiene par en en/de/main.ftl.

## PÃ­ldoras de estado de ValueSet.

hts-vs-status-draft = borrador
hts-vs-status-active = activo
hts-vs-status-retired = retirado
hts-vs-status-unknown = desconocido

## PÃ¡gina del navegador VS.

hts-vs-browser-title = Conjuntos de valores
hts-vs-browser-subtitle = Explora el catÃ¡logo de ValueSets del servidor de terminologÃ­a y abre cualquier fila para inspeccionar sus metadatos o ejecutar una expansiÃ³n.
hts-vs-browser-filter-legend = Filtrar ValueSets
hts-vs-browser-filter-url = URL canÃ³nica
hts-vs-browser-filter-version = VersiÃ³n
hts-vs-browser-filter-name = Nombre
hts-vs-browser-filter-title = TÃ­tulo
hts-vs-browser-filter-status = Estado
hts-vs-browser-filter-search = Buscar
hts-vs-browser-filter-reset = Restablecer
hts-vs-browser-empty = NingÃºn ValueSet coincide con estos filtros.
hts-vs-browser-load-more = Cargar mÃ¡s
hts-vs-browser-showing-count = Mostrando { $count ->
    [one] { $count } ValueSet
   *[other] { $count } ValueSets
}
hts-vs-browser-table-caption = ValueSets que coinciden con los filtros activos.
hts-vs-browser-column-url = URL
hts-vs-browser-column-version = VersiÃ³n
hts-vs-browser-column-title = TÃ­tulo
hts-vs-browser-column-status = Estado

## PÃ¡gina de detalle VS.

hts-vs-detail-title = { $name } Â· ValueSet
hts-vs-detail-title-fallback = ValueSet
hts-vs-detail-eyebrow = ValueSet
hts-vs-detail-section-identity = Identidad
hts-vs-detail-section-governance = Gobernanza
hts-vs-detail-publisher = Publicador
hts-vs-detail-jurisdiction = JurisdicciÃ³n
hts-vs-detail-immutable = Inmutable
hts-vs-detail-immutable-yes = sÃ­
hts-vs-detail-immutable-no = no
hts-vs-detail-purpose = PropÃ³sito
hts-vs-detail-copyright = Derechos de autor
hts-vs-detail-tabs-label = Secciones del banco de trabajo del ValueSet
hts-vs-detail-tab-metadata = Metadatos
hts-vs-detail-tab-expand = Expandir
hts-vs-detail-workbench-hint = Elige una operaciÃ³n para ejecutarla sobre este ValueSet.
hts-vs-detail-result-empty = Ejecuta la operaciÃ³n para ver su resultado aquÃ­.

## Formulario y resultados de $expand.

hts-vs-expand-heading = Expandir este ValueSet
hts-vs-expand-scoped-valueset = ValueSet (fijo)
hts-vs-expand-filter = Filtro
hts-vs-expand-filter-placeholder = cÃ³digo o texto de visualizaciÃ³n
hts-vs-expand-count = count
hts-vs-expand-offset = offset
hts-vs-expand-display-language = Idioma de visualizaciÃ³n
hts-vs-expand-display-language-placeholder = p. ej. es-ES
hts-vs-expand-flags-legend = Opciones
hts-vs-expand-active-only = Solo conceptos activos
hts-vs-expand-include-designations = Incluir designaciones
hts-vs-expand-mode-legend = Modo del resultado
hts-vs-expand-mode-flat = Plano
hts-vs-expand-mode-tree = Ãrbol
hts-vs-expand-use-supplement-legend = Suplementos aplicados
hts-vs-expand-use-supplement-placeholder = URL canÃ³nica
hts-vs-expand-advanced-summary = Avanzado
hts-vs-expand-date = Fecha
hts-vs-expand-date-placeholder = ISO 8601 (p. ej. 2025-06-01)
hts-vs-expand-property-legend = Propiedades
hts-vs-expand-property-placeholder = cÃ³digo de propiedad
hts-vs-expand-tx-resource-legend = tx-resource
hts-vs-expand-tx-resource-placeholder = URL canÃ³nica o referencia
hts-vs-expand-system-version-legend = system-version
hts-vs-expand-system-version-placeholder = sistema|versiÃ³n
hts-vs-expand-check-system-version-legend = check-system-version
hts-vs-expand-force-system-version-legend = force-system-version
hts-vs-expand-default-valueset-version = default-valueset-version
hts-vs-expand-threshold = Umbral too-costly
hts-vs-expand-ceiling-tooltip = LÃ­mite superior de la UI: { $ceiling } (valores mayores se descartan)
hts-vs-expand-ceiling-note = lÃ­mite: { $ceiling }
hts-vs-expand-ceiling-warning-title = Umbral por encima del lÃ­mite de la UI
hts-vs-expand-ceiling-warning-body = Solicitaste el umbral { $requested }, que supera el lÃ­mite de la UI â€” la cabecera no se adjuntÃ³.
hts-vs-expand-ceiling-value = lÃ­mite: { $ceiling }
hts-vs-expand-too-costly-title = ExpansiÃ³n rechazada por costosa
hts-vs-expand-too-costly-body = HTS rechazÃ³ la expansiÃ³n por superar el umbral actual. SÃºbelo aquÃ­ abajo y reintenta, o restringe el filtro.
hts-vs-expand-raise-threshold = Elevar umbral a
hts-vs-expand-raise-submit = Reintentar
hts-vs-expand-tree-label = mostrando el Ã¡rbol completo { $count ->
    [one] { $count } hoja
   *[other] { $count } hojas
}
hts-vs-expand-total-label = total { $total }
hts-vs-expand-total-unknown = total (desconocido)
hts-vs-expand-offset-label = offset { $offset }
hts-vs-expand-filter-no-match = NingÃºn miembro coincide con el filtro "{ $filter }".
hts-vs-expand-no-members = Esta expansiÃ³n no contiene miembros.
hts-vs-expand-column-code = CÃ³digo
hts-vs-expand-column-display = VisualizaciÃ³n
hts-vs-expand-column-system = Sistema
hts-vs-expand-load-more = Cargar mÃ¡s
hts-vs-expand-echoed-parameters = ParÃ¡metros ecoados

## HTS Slice D â€” Explorador y detalle de ConceptMap con banco de trabajo
## de $translate embebido (doc. de diseÃ±o Â§7.5). Cada clave tiene su par
## en en/de/main.ftl.

## Estados del ConceptMap.

hts-cm-status-draft = borrador
hts-cm-status-active = activo
hts-cm-status-retired = retirado
hts-cm-status-unknown = desconocido

## Explorador de CM.

hts-cm-browser-title = Mapas de conceptos
hts-cm-browser-subtitle = Explora el catÃ¡logo de ConceptMaps del servidor de terminologÃ­a y abre cualquier fila para inspeccionar sus metadatos o ejecutar una traducciÃ³n.
hts-cm-browser-filter-legend = Filtrar ConceptMaps
hts-cm-browser-filter-url = URL canÃ³nica
hts-cm-browser-filter-name = Nombre
hts-cm-browser-filter-title = TÃ­tulo
hts-cm-browser-filter-source = Sistema origen
hts-cm-browser-filter-target = Sistema destino
hts-cm-browser-filter-status = Estado
hts-cm-browser-filter-search = Buscar
hts-cm-browser-filter-reset = Restablecer
hts-cm-browser-empty = NingÃºn ConceptMap coincide con estos filtros.
hts-cm-browser-load-more = Cargar mÃ¡s
hts-cm-browser-showing-count = Mostrando { $count ->
    [one] { $count } ConceptMap
   *[other] { $count } ConceptMaps
}
hts-cm-browser-table-caption = ConceptMaps que coinciden con los filtros activos.
hts-cm-browser-column-url = URL
hts-cm-browser-column-version = VersiÃ³n
hts-cm-browser-column-title = TÃ­tulo
hts-cm-browser-column-status = Estado

## Detalle de CM.

hts-cm-detail-title = { $name } Â· ConceptMap
hts-cm-detail-title-fallback = ConceptMap
hts-cm-detail-eyebrow = ConceptMap
hts-cm-detail-section-identity = Identidad
hts-cm-detail-section-mapping = Mapeo
hts-cm-detail-publisher = Publicador
hts-cm-detail-jurisdiction = JurisdicciÃ³n
hts-cm-detail-purpose = PropÃ³sito
hts-cm-detail-source-uri = Origen
hts-cm-detail-target-uri = Destino
hts-cm-detail-group-count = Grupos
hts-cm-detail-tabs-label = Secciones del banco de trabajo del ConceptMap
hts-cm-detail-tab-metadata = Metadatos
hts-cm-detail-tab-translate = Traducir
hts-cm-detail-workbench-hint = Elige una operaciÃ³n para ejecutarla sobre este ConceptMap.
hts-cm-detail-result-empty = Ejecuta la operaciÃ³n para ver su resultado aquÃ­.

## Formulario y resultados de $translate.

hts-cm-translate-heading = Traducir un cÃ³digo
hts-cm-translate-scoped-map = ConceptMap (fijo)
hts-cm-translate-direction-legend = DirecciÃ³n
hts-cm-translate-direction-forward = Directa
hts-cm-translate-direction-reverse = Inversa
hts-cm-translate-source-legend = CodificaciÃ³n origen
hts-cm-translate-source-system = Sistema
hts-cm-translate-source-system-placeholder = URL canÃ³nica
hts-cm-translate-source-code = CÃ³digo
hts-cm-translate-source-display = VisualizaciÃ³n
hts-cm-translate-source-display-placeholder = opcional
hts-cm-translate-reverse-legend = Origen inverso
hts-cm-translate-target-code = CÃ³digo destino
hts-cm-translate-target-code-hint = Obligatorio en modo inverso.
hts-cm-translate-target-legend = Restricciones de destino
hts-cm-translate-target-system = Sistema destino
hts-cm-translate-target-system-placeholder = URL canÃ³nica
hts-cm-translate-source-url = ValueSet origen
hts-cm-translate-source-url-placeholder = URL canÃ³nica (opcional)
hts-cm-translate-target-url = ValueSet destino
hts-cm-translate-target-url-placeholder = URL canÃ³nica (opcional)
hts-cm-translate-date = Fecha
hts-cm-translate-date-placeholder = ISO 8601 (p. ej. 2025-06-01)
hts-cm-translate-submit = Traducir
hts-cm-translate-matches-heading = Coincidencias
hts-cm-translate-matches-count = { $count ->
    [one] { $count } coincidencia
   *[other] { $count } coincidencias
}
hts-cm-translate-no-matches = No hay coincidencias para este origen.
hts-cm-translate-column-code = CÃ³digo
hts-cm-translate-column-system = Sistema
hts-cm-translate-column-display = VisualizaciÃ³n
hts-cm-translate-column-mapping = { $kind ->
    [equivalence] Equivalencia
    [relationship] RelaciÃ³n
   *[other] Mapeo
}
hts-cm-translate-column-origin = Origen
hts-cm-translate-column-mapping-equivalence = Equivalencia
hts-cm-translate-column-mapping-relationship = RelaciÃ³n
hts-cm-translate-validate-forward-missing = La traducciÃ³n directa requiere ambos `code` y `system`.
hts-cm-translate-validate-reverse-missing-target-code = La traducciÃ³n inversa requiere `targetCode`.

## HTS Slice E -- operaciones (design doc s7.6).

hts-operations-title = Banco de operaciones
hts-operations-eyebrow = Terminologia
hts-operations-subtitle = Ejecuta operaciones de terminologia contra el servidor. Cada operacion se envia como POST sin importar el verbo del formulario.
hts-operations-selector-label = Operacion
hts-operations-resource-tabs-label = Familia de recurso
hts-operations-resource-code-system = CodeSystem
hts-operations-resource-value-set = ValueSet
hts-operations-result-empty = Ejecuta la operacion para ver el resultado aqui.
hts-operations-scope-legend = Ambito
hts-operations-scope-system = URL canonica de CodeSystem
hts-operations-scope-instance = Id de instancia
hts-operations-scope-instance-placeholder = id de la instancia
hts-operations-scope-canonical = URL canonica
hts-operations-not-implemented = Esta operacion llega en la fase E2.
hts-operations-closure-stateless-warning = El estado del cierre vive en el servidor bajo el `name` proporcionado. La UI no lo persiste entre peticiones.
hts-operations-closure-empty-graph = Aun no hay aristas -- envia al menos un Coding para agregar nodos al grafo.

hts-operations-op-lookup = $lookup
hts-operations-op-validate-code = $validate-code
hts-operations-op-subsumes = $subsumes
hts-operations-op-expand = $expand
hts-operations-op-translate = $translate
hts-operations-op-closure = $closure
hts-operations-op-batch-validate = batch-validate

hts-cs-lookup-useSupplement = Suplemento
hts-cs-lookup-useSupplement-hint = URL canonica opcional de un suplemento de CodeSystem para superponer sobre la base.
hts-cs-lookup-result-heading = Resultado del lookup
hts-cs-lookup-fact-name = Nombre
hts-cs-lookup-fact-version = Version
hts-cs-lookup-fact-display = Etiqueta
hts-cs-lookup-fact-definition = Definicion

hts-cs-validate-version = Version del CodeSystem
hts-cs-validate-systemVersion = Sobrescribir la version del sistema
hts-cs-validate-mode-CodeableConcept = CodeableConcept
hts-cs-validate-displayLanguage = Idioma de la etiqueta
hts-cs-validate-advanced = Parametros avanzados
hts-cs-validate-date = Fecha
hts-cs-validate-activeOnly = Solo codigos activos
hts-cs-validate-abstract = Permitir codigos abstractos
hts-cs-validate-lenient-display-validation = Validacion de etiqueta laxa
hts-cs-validate-useSupplement = URL del suplemento
hts-cs-validate-system-version = Fijar version del sistema
hts-cs-validate-check-system-version = Verificar version del sistema
hts-cs-validate-force-system-version = Forzar version del sistema
hts-cs-validate-result-heading = Resultado de la validacion
hts-cs-validate-result-badge-true = Valido
hts-cs-validate-result-badge-false = No valido
hts-cs-validate-fact-code = Codigo
hts-cs-validate-fact-display = Etiqueta
hts-cs-validate-fact-message = Mensaje

hts-cs-subsumes-version = Version
hts-cs-subsumes-codeA = Codigo A
hts-cs-subsumes-codeB = Codigo B
hts-cs-subsumes-result-heading = Resultado de la subsuncion

hts-vs-expand-displayLanguage = Idioma de la etiqueta
hts-vs-expand-activeOnly = Solo activos
hts-vs-expand-includeDesignations = Incluir designaciones
hts-vs-expand-designation = Designacion
hts-vs-expand-designation-hint = Chip filtro -- un par `use|value` por linea (repetible).
hts-vs-expand-advanced = Parametros avanzados
hts-vs-expand-threshold-hint = HTS rechaza expansiones sobre el umbral. Tope de la UI: { $ceiling }.
hts-vs-expand-result-heading = Expansion
hts-vs-expand-total = total { $n }
hts-vs-expand-count-shown = mostrando { $n }

hts-vs-validate-heading = Validar un codigo contra un ValueSet
hts-vs-validate-source-legend = Fuente del ValueSet
hts-vs-validate-source-canonical = URL canonica
hts-vs-validate-source-instance = Id de instancia
hts-vs-validate-source-inline = JSON en linea
hts-vs-validate-mode-legend = Forma del input
hts-vs-validate-mode-code = Code
hts-vs-validate-mode-coding = Coding
hts-vs-validate-mode-CodeableConcept = CodeableConcept
hts-vs-validate-code = Codigo
hts-vs-validate-system = System
hts-vs-validate-systemVersion = Version del sistema
hts-vs-validate-display = Etiqueta
hts-vs-validate-coding-legend = Coding
hts-vs-validate-coding-system = System
hts-vs-validate-coding-code = Code
hts-vs-validate-coding-display = Display
hts-vs-validate-displayLanguage = Idioma de la etiqueta
hts-vs-validate-valueSetVersion = Version del ValueSet
hts-vs-validate-advanced = Parametros avanzados
hts-vs-validate-date = Fecha
hts-vs-validate-activeOnly = Solo activos
hts-vs-validate-abstract = Permitir codigos abstractos
hts-vs-validate-lenient-display-validation = Validacion de etiqueta laxa
hts-vs-validate-useSupplement = URL del suplemento
hts-vs-validate-tx-resource = tx-resource extra
hts-vs-validate-default-valueset-version = Version predeterminada del ValueSet
hts-vs-validate-no-membership = El codigo no pertenece al ValueSet.
hts-vs-validate-result-heading = Resultado de la validacion
hts-vs-validate-result-badge-true = Valido
hts-vs-validate-result-badge-false = No valido
hts-vs-validate-fact-code = Codigo
hts-vs-validate-fact-system = System
hts-vs-validate-fact-display = Etiqueta
hts-vs-validate-fact-message = Mensaje

hts-cm-translate-code = Codigo
hts-cm-translate-system = Sistema
hts-cm-translate-display = Etiqueta
hts-cm-translate-targetCode = Codigo destino
hts-cm-translate-targetSystem = Sistema destino
hts-cm-translate-result-heading = Resultado de la traduccion
hts-cm-translate-result-badge-true = Coincide
hts-cm-translate-result-badge-false = Sin coincidencia

hts-cm-closure-heading = Grafo de cierre
hts-cm-closure-name = Nombre del cierre
hts-cm-closure-name-hint = Nombre proporcionado por el cliente que identifica la tabla de cierre en el servidor entre peticiones.
hts-cm-closure-concepts-legend = Conceptos
hts-cm-closure-concepts-hint = Agrega hasta tres codings semilla; cada fila es un par sistema + codigo.
hts-cm-closure-concept-system = Sistema
hts-cm-closure-concept-code = Codigo
hts-cm-closure-result-heading = Aristas del cierre
hts-cm-closure-edge-source = Origen
hts-cm-closure-edge-equivalence = Equivalencia
hts-cm-closure-edge-target = Destino

hts-vs-batch-heading = Validar codigos en lote contra un ValueSet
hts-vs-batch-target-value-set-label = ValueSet destino
hts-vs-batch-rows-legend = Filas
hts-vs-batch-rows-hint = Ingresa un codigo por fila; las filas vacias se descartan.
hts-vs-batch-row-code = Codigo
hts-vs-batch-row-system = Sistema
hts-vs-batch-row-display = Etiqueta
hts-vs-batch-row-timeout = Excedio el tiempo
hts-vs-batch-row-placeholder = --
hts-vs-batch-result-heading = Resultado del lote
hts-vs-batch-target-hint = ValueSet destino: { $target }
hts-vs-batch-column-code = Codigo
hts-vs-batch-column-system = Sistema
hts-vs-batch-column-display = Etiqueta
hts-vs-batch-column-result = Resultado
hts-vs-batch-progress = { $n } de { $m } completadas
hts-vs-batch-progress-final = { $m } completadas
