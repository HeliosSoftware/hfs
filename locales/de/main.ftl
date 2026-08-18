# Helios FHIR-Server â€” UI-Nachrichtenkatalog
# Gebietsschema: Deutsch (de)
#
# Verwenden Sie dieselben SchlÃ¼ssel wie in `en/main.ftl` (Quell-Gebietsschema).
# Fehlende SchlÃ¼ssel greifen gemÃ¤ÃŸ der in docs/multi-language.md beschriebenen
# Fallback-Kette auf Englisch zurÃ¼ck.

## Marke / gemeinsame Begriffe

-app-name = Helios FHIR-Server
-org-name = Helios Software

## Seitenstruktur

app-title = { -app-name }
app-tagline = Ein schneller, versionsÃ¼bergreifender FHIR-Server

nav-dashboard = Ãœbersicht
nav-terminology = Terminologie
nav-resources = Ressourcen
nav-settings = Einstellungen
nav-signout = Abmelden

## Sprachauswahl

language-label = Sprache
language-en = Englisch
language-es = Spanisch
language-de = Deutsch

## Startseite

home-lede = Serverseitig gerenderte, HTMX-basierte OberflÃ¤che. Dieses Panel wird als HTML-Fragment aktualisiert.

## Statuspanel

status-last-checked = Zuletzt geprÃ¼ft: { $timestamp }

## Ãœbersicht / Status

dashboard-heading = Server-Ãœbersicht
health-status-ok = Alle Systeme betriebsbereit
health-status-degraded = Einige Systeme sind beeintrÃ¤chtigt
health-uptime = Betriebszeit: { $duration }

resource-count = { $count ->
    [one] { $count } Ressource
   *[other] { $count } Ressourcen
}

## Terminologie durchsuchen

terminology-search-label = CodeSystems und ValueSets durchsuchen
terminology-search-placeholder = z. B. 73211009, â€žDiabetesâ€œ, http://snomed.info/sct
terminology-display-language = Anzeigesprache
terminology-no-results = Keine passenden Konzepte gefunden.

## Allgemeine Aktionen

action-search = Suchen
action-save = Speichern
action-cancel = Abbrechen
action-retry = Erneut versuchen

## Fehler (spiegelt den OperationOutcome-Text wider; siehe docs/multi-language.md Â§5)

error-not-found = Die angeforderte Ressource wurde nicht gefunden.
error-unauthorized = Sie sind nicht berechtigt, diese Aktion auszufÃ¼hren.
error-generic = Etwas ist schiefgelaufen. Bitte versuchen Sie es erneut.

## Dashboard-GerÃ¼st (Figma â€žDashboard V1.1â€œ)

nav-section-work = Arbeit
nav-section-batch-data = Batch & Daten
nav-section-server = Server
nav-section-conditional = Bedingt

nav-home = Startseite
nav-search = Suche
nav-resource-editor = Ressourcen-Editor
nav-history-versions = Verlauf & Versionen
nav-compartments = Compartments
nav-batch-transaction = Batch / Transaktion
nav-import = Importieren
nav-export = Exportieren
nav-sql-on-fhir = SQL-on-FHIR
nav-capability-conformance = Capability & KonformitÃ¤t
nav-search-parameters = Suchparameter
nav-admin-ops = Admin / Betrieb
nav-subscriptions = Abonnements
nav-tenants = Mandanten

## Mandantenverwaltung (/ui/tenants)

tenants-title = Mandantenverwaltung
tenants-unavailable = Die Mandantenregistrierung ist auf diesem Speicher-Backend nicht verfÃ¼gbar.
tenants-stat-total = Mandanten gesamt
tenants-stat-total-sub = { $count ->
    [one] { $count } registriert
   *[other] { $count } registriert
}
tenants-stat-resources = Gespeicherte Ressourcen
tenants-stat-resources-sub = Ã¼ber alle Mandanten
tenants-search-placeholder = Nach Name oder Mandanten-ID suchenâ€¦
tenants-add = Mandant hinzufÃ¼gen
tenants-add-title = Einen Mandanten hinzufÃ¼gen
tenants-field-id = Mandanten-ID
tenants-field-id-hint = Wird in der API verwendet (Header X-Tenant-ID, URL-PrÃ¤fix, JWT-Claim).
tenants-field-name = Anzeigename (optional)
tenants-field-name-hint = Eine lesbare Bezeichnung; nicht fÃ¼r das Routing verwendet.
tenants-add-submit = Mandant bereitstellen
tenants-col-tenant = Mandant
tenants-col-resources = Ressourcen
tenants-col-created = Erstellt
tenants-col-actions = Aktionen
tenants-empty = Keine Mandanten gefunden.
tenants-unregistered = nicht registriert
tenants-delete = Mandant lÃ¶schen
tenants-delete-confirm = Mandant â€ž{ $id }" abmelden? Die gespeicherten Daten bleiben erhalten, sofern sie nicht Ã¼ber die API bereinigt werden.

tenant-heading = Tenants
tenant-all = Alle Tenants
tenant-search-placeholder = Tenants durchsuchen

theme-label = Farbschema
theme-light = Helles Design
theme-dark = Dunkles Design

fhir-version = FHIR { $version }
fhir-version-heading = FHIR-Version

card-resource-types = Ressourcentypen
card-resource-types-sub = aktiviert fÃ¼r { $version }
card-stored-resources = Gespeicherte Ressourcen
card-stored-resources-sub = im aktiven Tenant
card-export-jobs = Export-Jobs
card-export-jobs-sub = laufend ({ $queued } in der Warteschlange)
card-uptime = VerfÃ¼gbarkeit
card-uptime-sub = letzte 30 Tage

chart-title = FHIR-Ressourcen im Zeitverlauf
chart-expand = Diagramm vergrÃ¶ÃŸern
chart-window = Zeitfenster des Diagramms

## FuÃŸzeile

footer-copyright = Â© { $year } { -org-name }

## Verlauf & Versionen (#236)

history-heading = Verlauf & Versionen
history-lede = Zwei Versionen einer Ressource vergleichen. Der Speicher ist vollstÃ¤ndig versioniert; dies liest ihn Ã¼ber die Ã¼bliche _history- und vread-API.
history-type-label = Ressourcentyp
history-id-label = Ressourcen-ID
history-id-placeholder = Ressourcen-ID
history-load = Laden
history-tabs-label = Verlaufsbereich
history-tab-instance = Instanz
history-tab-type = Typ-Feed
history-tab-system = System-Feed
history-versions-label = Versionen
history-pick-instance = Instanz wÃ¤hlen
history-current = aktuell
history-from = Von
history-to = Bis
history-show-metadata = MetadatenÃ¤nderungen anzeigen
history-empty = Laden Sie eine Ressource und wÃ¤hlen Sie zwei Versionen zum Vergleich.
history-load-error = Der Verlauf dieser Ressource konnte nicht geladen werden.
history-not-found = Kein Verlauf fÃ¼r diese Ressource â€” Typ und ID prÃ¼fen.
history-diff-heading = { $from }
history-metadata-hidden = { $count ->
    [one] { $count } MetadatenÃ¤nderung ausgeblendet
   *[other] { $count } MetadatenÃ¤nderungen ausgeblendet
}
history-textual = VollstÃ¤ndigen Text-Diff anzeigen
history-only-metadata = Zwischen diesen Versionen Ã¤nderten sich nur die Metadaten.
history-identical = Diese beiden Versionen sind identisch.
history-deleted = { $version } ist eine LÃ¶schung â€” es gibt nichts zu vergleichen.
history-parse-error = Diese Versionen konnten nicht als JSON gelesen werden.
## Saved queries (#234)

nav-saved-queries = Gespeicherte Abfragen

queries-heading = Gespeicherte Abfragen
queries-lede = FHIR-Suchabfragen je Ressourcentyp aufbewahren, sortiert nach der letzten AusfÃ¼hrung. Sie werden in deinen Benutzereinstellungen gespeichert und stehen auf allen GerÃ¤ten bereit.
queries-add-heading = Abfrage speichern
queries-type-label = Ressourcentyp
queries-type-placeholder = z. B. Patient
queries-name-label = Name
queries-name-placeholder = z. B. Smiths in Boston
queries-query-label = Abfrage
queries-query-placeholder = z. B. name=smith&address-city=Boston
queries-empty = Noch keine gespeicherten Abfragen. Speichere oben eine, um loszulegen.
queries-never-run = Nie ausgefÃ¼hrt
queries-run = AusfÃ¼hren
queries-rename = Umbenennen
queries-delete = LÃ¶schen
queries-rename-prompt = Neuer Name
queries-confirm-delete = â€ž{ $name }â€œ lÃ¶schen?
queries-unavailable = Gespeicherte Abfragen sind nicht verfÃ¼gbar: Das Storage-Backend dieses Servers unterstÃ¼tzt keine Benutzereinstellungen.

## SearchParameter-Ansicht (#238)

sp-heading = Suchparameter
sp-lede = Durchsuche die Parameter, mit denen dieser Server Suchen auflÃ¶st, gefiltert nach Basis-Ressourcentyp. Gespeicherte Parameter lassen sich anlegen, bearbeiten und lÃ¶schen; die Registry Ã¼bernimmt Ã„nderungen pro Tenant.
sp-version-label = FHIR-Version
sp-spec-missing = Das vollstÃ¤ndige Spezifikations-Bundle (search-parameters-*.json) wurde im Datenverzeichnis nicht gefunden â€” es werden nur die minimalen eingebetteten Fallback-Parameter angezeigt.
sp-rail-label = Ressourcenfilter
sp-rail-search = Typen filtern
sp-rail-recent = Zuletzt verwendet
sp-rail-types = Ressourcentypen
sp-rail-all = Alle Typen
sp-facet-type = Typ
sp-facet-type-label = Nach Parametertyp filtern
sp-facet-source = Quelle
sp-facet-source-label = Nach Quelle filtern
sp-source-embedded = eingebettet
sp-source-stored = gespeichert
sp-source-config = Konfiguration
sp-chip-conflict = Konflikt
sp-chip-overrides = Ã¼berschreibt Spez.
sp-chip-shadowed = verdeckt
sp-col-code = Code
sp-col-type = Typ
sp-col-base = Basis
sp-col-expression = Ausdruck
sp-col-source = Quelle
sp-total = { $count } Parameter
sp-pagination-label = Seiten
sp-page-prev = ZurÃ¼ck
sp-page-next = Weiter
sp-detail-label = Parameterdetails
sp-detail-empty = Kein Parameter ausgewÃ¤hlt
sp-detail-empty-hint = WÃ¤hle eine Zeile, um Definition, Ausdruck und die AuflÃ¶sung im Register zu prÃ¼fen.
sp-detail-readonly = Spezifikationsparameter (aus der Datendatei einkompiliert) â€” schreibgeschÃ¼tzt.
sp-field-url = Kanonische URL
sp-field-name = Name
sp-field-status = Status
sp-field-base = Basis-Ressourcentypen
sp-field-expression = FHIRPath-Ausdruck
sp-field-description = Beschreibung
sp-field-target = Zieltypen
sp-field-components = Komponenten
sp-status-hint = Der Loader stuft den Draft-Status der Spezifikation beim Laden auf active hoch.
sp-note-conflict = Doppeltes (base, code) innerhalb derselben Quelle wie { $url } â€” das Register lehnt diese Kollision ab (DuplicateCode).
sp-note-overrides = Ãœberschreibt { $url } auf (base, code): eine gespeicherte Definition hat Vorrang vor dem Spezifikationsparameter und lÃ¶st daher die Suchen auf. Das Register loggt ein WARN mit beiden URLs.
sp-note-shadowed = Verdeckt durch { $url } auf (base, code): eine Quelle mit hÃ¶herem Vorrang lÃ¶st die Suchen fÃ¼r diesen Slot auf.
sp-note-empty-expression = Leerer Ausdruck: der Extractor indexiert keine Zeilen, jede Suche Ã¼ber diesen Parameter liefert stillschweigend nichts.
sp-note-no-target = Referenzparameter ohne Zieltypen: verkettete Suche kann den referenzierten Typ nicht auflÃ¶sen.
sp-note-choice-type = Choice-Typ-Ausdruck: der Extractor schreibt ofType(T) / as T vor der Auswertung gegen das gespeicherte JSON auf das konkrete Element um (z. B. valueQuantity).
sp-new = Neuer Suchparameter
sp-edit = Bearbeiten
sp-delete = LÃ¶schen
sp-delete-confirm = Diesen gespeicherten Suchparameter lÃ¶schen? Suchen, die ihn verwenden, finden nach der Aktualisierung der Registry keine Treffer mehr.
cmp-new = Neue Compartment-Definition
cmp-edit = Bearbeiten
cmp-delete = LÃ¶schen
cmp-delete-confirm = Diese Compartment-Definition lÃ¶schen? Ihre Compartment-Routen funktionieren dann nicht mehr.
crud-delete-failed = LÃ¶schen fehlgeschlagen

## Compartment-Ansicht & Tester (#237)

cmp-heading = Compartments
cmp-lede = Die Compartment-Definitionen, mit denen dieser Server /{"{"}compartment{"}"}/{"{"}id{"}"}/{"{"}type{"}"}-Anfragen routet, und ein Tester, der beantwortet: Ist dieser Typ in diesem Compartment, Ã¼ber welche Parameter, und welche Suche fÃ¼hrt der Server aus?
cmp-rail-label = Compartment-Definitionen
cmp-rail-heading = Compartments
cmp-degraded = Die Compartment-Definitionen konnten gerade nicht von diesem Server geladen werden â€” der Selbstaufruf an /CompartmentDefinition schlug fehl (bei aktivierter Authentifizierung fehlt meist das ausgehende Service-Token oder es ist ungÃ¼ltig). Die Seite versucht es bei der nÃ¤chsten Anfrage erneut.
cmp-rail-note = Die Definitionen sind gespeicherte Ressourcen, beim Start aus der FHIR-Spezifikation angelegt. Bearbeiten und LÃ¶schen wirken hier pro Tenant.
cmp-tabs-label = Compartment-Bereiche
cmp-tab-definition = Definition
cmp-tab-members = Mitglieder
cmp-tab-tester = Tester
cmp-field-code = Code
cmp-field-status = Status
cmp-field-url = Kanonische URL
cmp-field-version = Version
cmp-field-publisher = Herausgeber
cmp-field-description = Beschreibung
cmp-field-search = search
cmp-field-experimental = experimental
cmp-search-why = Aus wÃ¼rde bedeuten, dass keine Compartment-Route fÃ¼r dieses Compartment auflÃ¶st.
cmp-on = an
cmp-off = aus
cmp-yes = ja
cmp-no = nein
cmp-readonly-note = SchreibgeschÃ¼tzt: diese Werte stammen aus den in den Server einkompilierten Spezifikationsdefinitionen.
cmp-filter-members = Mitglieder
cmp-filter-all = Alle Typen
cmp-filter-excluded = Ausgeschlossen
cmp-member = Mitglied
cmp-excluded = ausgeschlossen
cmp-tester-id = Id
cmp-tester-target = Zieltyp (oder *)
cmp-tester-run = Testen
cmp-result-member = âœ“ Mitglied â€” Ã¼ber { $params }
cmp-result-flat = // Ã¤quivalente flache Suche
cmp-result-member-note = Der Server lÃ¶st die Compartment-Route zu dieser Suche Ã¼ber die Referenzparameter des Typs auf.
cmp-result-self = âœ“ Mitglied â€” die Compartment-Ressource selbst ({"{"}def{"}"})
cmp-result-self-note = Die Compartment-Instanz ist trivialerweise in ihrem eigenen Compartment; die Route liest die Ressource direkt.
cmp-result-notmember = âœ• { $type } ist kein Mitglied dieses Compartments
cmp-result-notmember-note = Der Server antwortet mit 404 und einem OperationOutcome fÃ¼r Typen, die keine Compartment-Mitglieder sind.
cmp-result-fanout = FÃ¤chert auf { $count } Mitgliedstypen auf
cmp-result-fanout-note = Ausgeschlossene Typen werden Ã¼bersprungen, nicht fehlgeschlagen â€” der Fan-out lÃ¤sst Nicht-Mitgliedstypen weg statt zu scheitern.
queries-builder-heading = Such-Builder
queries-url-label = FHIR-Such-URL
queries-url-placeholder = GET /Patient?name=smith&birthdate=ge1980-01-01
queries-builder-hint = Bearbeite die GET-URL direkt oder Ã¼ber die Zeilen darunter â€” beide bleiben synchron. AusfÃ¼hren fÃ¼hrt die Suche hier aus und trÃ¤gt sie unter â€žZuletzt" ein; mit einem Namen bleibt sie in der Liste gespeichert.
queries-recent = Zuletzt
queries-recent-heading = Letzte Suchen
queries-recent-empty = Noch keine letzten Suchen â€” fÃ¼hre eine aus, um sie hier einzutragen.
queries-invalid-url = Gib eine Suche wie GET /Patient?name=smith ein â€” der Ressourcentyp kommt aus dem Pfad.

queries-conditions = Bedingungen
queries-add-condition = Bedingung hinzufÃ¼gen
queries-includes = Includes
queries-result-controls = Ergebnis-Steuerung
queries-remove = Entfernen
queries-match-is = ist
queries-or = + oder
plain-pill = In einfachen Worten
plain-find = Finde {"{type}"}-EintrÃ¤ge
plain-clause = {"{path}"} {"{verb}"} {"{value}"}
plain-and = und
plain-or = oder
plain-arrow = {" "}â†’
plain-has = die ein verknÃ¼pftes {"{type}"} haben, dessen {"{param}"} {"{verb}"} {"{value}"}
plain-include = ZusÃ¤tzlich wird der {"{param}"} jedes {"{type}"} zurÃ¼ckgegeben{"{target}"}
plain-revinclude = Plus jedes {"{type}"}, dessen {"{param}"} hierher zeigt
plain-iterate = (wiederholt)
plain-count = Zeigt {"{n}"} pro Seite
plain-sort = Sortiert nach {"{sort}"}
plain-verb-is = ist
plain-verb-contains = enthÃ¤lt
plain-verb-exact = ist genau
plain-verb-missing = ist vorhanden/fehlt
plain-verb-not = ist nicht
plain-verb-text = entspricht dem Text
plain-verb-in = ist im Value Set
plain-verb-not-in = ist nicht im Value Set
plain-verb-identifier = hat den Identifier
plain-verb-of-type = hat einen Identifier vom Typ
plain-verb-ge = ist am oder nach
plain-verb-le = ist am oder vor
plain-verb-gt = ist nach
plain-verb-lt = ist vor
plain-verb-ne = ist nicht
plain-verb-eq = ist
plain-verb-sa = beginnt nach
plain-verb-eb = endet vor
plain-verb-ap = ist ungefÃ¤hr
queries-related-heading = Verwandte Daten einbeziehen
queries-related-sub = FÃ¼gt verbundene Ressourcen zu den Ergebnissen hinzu.
queries-related-add-include = Eine referenzierte Ressource einbeziehen
queries-related-add-revinclude = Hierher verweisende Ressourcen einbeziehen
queries-iterate = Iterieren
queries-sort-label = Sortierung
queries-sort-default = Standard
queries-sort-recent = Neueste zuerst
queries-sort-oldest = Ã„lteste zuerst
queries-sort-id = ID
queries-modify-heading = Modifikatoren
queries-mod-exact = ganzer Wert inkl. GroÃŸ-/Kleinschreibung & Akzente
queries-mod-contains = Treffer irgendwo im Text
queries-mod-missing = Feld ist vorhanden / fehlt
queries-mod-text = erweiterte Textbehandlung
queries-mod-not = keiner der Werte trifft zu
queries-mod-above = dieser oder ein Vorfahr
queries-mod-below = dieser oder ein Nachfahre
queries-mod-in = Mitglied des Value Sets
queries-mod-not-in = kein Mitglied des Value Sets
queries-mod-identifier = Referenz nach Identifier abgleichen
queries-mod-of-type = Identifier-Typ, -System und -Wert abgleichen
queries-chain-into = Nach einer Eigenschaft der referenzierten Ressource filtern
queries-chain-any-target = beliebig
queries-has-pill = hat eine verknÃ¼pfte
queries-has-type-placeholder = Ressourcentyp
queries-has-via = verknÃ¼pft Ã¼ber
queries-has-where = wobei ihr
queries-add-has = â§‰ Eine hierher verweisende Ressource filtern
queries-param-placeholder = Parameter
queries-value-placeholder = Wert
queries-results = Ergebnisse
queries-results-total = { $count } Ergebnisse
queries-results-included = { $count } eingeschlossen
queries-results-empty = Keine Ergebnisse.
queries-open-tab = In neuem Tab Ã¶ffnen
queries-col-updated = Aktualisiert
queries-prev = ZurÃ¼ck
queries-next = Weiter

queries-rail-heading = Ressourcentypen
queries-rail-filter = Typen filtern

## Suche â€” natÃ¼rliche Sprache & visueller Builder (#255)

search-heading = Suche
search-lede = Beschreiben Sie, wonach Sie suchen, oder bauen Sie die Abfrage selbst. So oder so erhalten Sie eine FHIR-Suchabfrage, die Sie lesen, korrigieren und ausfÃ¼hren kÃ¶nnen.
search-query-tag = ABFRAGE
search-copy = Abfrage kopieren

search-mode-label = Wie die Abfrage entsteht
search-mode-nl = NatÃ¼rliche Sprache
search-mode-builder = Visueller Builder

search-nl-label = Suche beschreiben
search-nl-placeholder = Beschreiben Sie, wonach Sie suchen â€” z. B. Patienten namens Smith, geboren nach 1980
search-nl-hint = Ihr Text und die Suchparameter dieses Servers gehen an das Sprachmodell. Patientendaten niemals. Die erzeugte Abfrage wird unten angezeigt â€” zum PrÃ¼fen und AusfÃ¼hren.
search-nl-working = Wird Ã¼bersetztâ€¦
search-nl-caveats = Wichtig zu wissen:
search-nl-unsupported = Das ist keine Suche, die dieser Server ausfÃ¼hren kann. Beschreiben Sie die DatensÃ¤tze, die Sie finden mÃ¶chten.

search-nl-example-1 = Weibliche Patientinnen Ã¼ber 65 mit Diabetes-Diagnose
search-nl-example-2 = Beobachtungen der letzten 30 Tage, neueste zuerst
search-nl-example-3 = Laufende FÃ¤lle im Boston General

search-setup-heading = Suche in natÃ¼rlicher Sprache ist verfÃ¼gbar
search-setup-body = Verwandelt Beschreibungen in Alltagssprache in FHIR-Suchabfragen. DafÃ¼r wird ein API-SchlÃ¼ssel fÃ¼r ein Sprachmodell benÃ¶tigt â€” der Server liest ihn aus der Umgebung, und er gelangt nie auf diese Seite. Bis einer gesetzt ist, nutzen Sie den visuellen Builder unten.
search-setup-key-placeholder = Ihr API-SchlÃ¼ssel
search-setup-disable = Um die Funktion vollstÃ¤ndig zu entfernen â€” Endpunkt, Seite und diesen Hinweis â€” setzen Sie HFS_NL_SEARCH_ENABLED=false.
search-setup-docs = Anleitung lesen

## Ressourcen-Editor (#264)

editor-heading = Ressourcen-Editor
editor-lede = Bearbeiten Sie eine Ressource anhand ihres Schemas: fÃ¼gen Sie jedes vom Schema erlaubte Element in beliebiger Tiefe hinzu â€” auch Extensions, an jedem Knoten, der sie zulÃ¤sst.
editor-title = Ressource bearbeiten
editor-view-label = Bearbeitungsmodus
editor-view-form = GefÃ¼hrtes Formular
editor-view-json = JSON
editor-save = Ã„nderungen speichern
editor-delete = LÃ¶schen
editor-remove = Diesen Knoten entfernen
editor-saved = Gespeichert.
editor-load-error = Diese Ressource konnte nicht geladen werden.
editor-confirm-delete = Diese Ressource lÃ¶schen? Das lÃ¤sst sich nicht rÃ¼ckgÃ¤ngig machen.
editor-invalid-json = Das ist kein gÃ¼ltiges JSON und kann daher nicht als Formular bearbeitet werden. Ihr Text bleibt unverÃ¤ndert.
editor-source-hint = Bearbeiten Sie den Quelltext direkt. Beim ZurÃ¼ckwechseln wird er geparst.

editor-add = Element hinzufÃ¼gen
editor-must-support-badge = MS
editor-binding-hint = An ein Value Set gebunden â€” Codes stammen daraus; StÃ¤rke angezeigt
editor-legend-live = Beim Tippen geprÃ¼ft: Struktur, KardinalitÃ¤t, erforderliche Bindings
editor-legend-save = Beim Speichern geprÃ¼ft: Constraints und Terminologie
editor-deferred-badge = beim Speichern
editor-deferred-hint = Codes werden beim Speichern gegen das Value Set geprÃ¼ft (und live im Picker, wenn ein Terminologieserver konfiguriert ist)
editor-must-support-hint = Must-support: Konsumenten dieses Profils mÃ¼ssen dieses Element verarbeiten kÃ¶nnen
editor-add-filter = Elemente filtern
editor-add-another = weiteres hinzufÃ¼gen
editor-pick-type = Typ wÃ¤hlenâ€¦
editor-extension-url = Extension-URL
editor-add-extension = Extension hinzufÃ¼gen

editor-valid = Keine Probleme.
editor-issues = { $count ->
    [one] { $count } Problem
   *[other] { $count } Probleme
}

editor-modifier-badge = Modifier
editor-modifier-warning = Eine Modifier-Extension Ã¤ndert die Bedeutung der Ressource. Ein System, das sie nicht kennt, muss die Verarbeitung verweigern.
editor-unknown-badge = nicht im Schema
editor-unknown-hint = Das Schema beschreibt dieses Element nicht. Es wird angezeigt, damit es nicht stillschweigend verloren geht, und beim Speichern erhalten.

editor-primitive-extension-badge = + Extension
editor-primitive-extension-hint = Dieser Wert trÃ¤gt eigene Extensions (ein `_`-Geschwister im JSON). Sie bleiben beim Speichern erhalten.

editor-collapse-all = Alle einklappen
editor-expand-all = Alle ausklappen
editor-edit-raw = Rohtext bearbeiten
editor-versions = Versionen
editor-versions-none = Keine frÃ¼heren Versionen.
## Verlauf & Versionen (#236)

## Ressourcen-Arbeitsbereich (#282)

resources-heading = Ressourcen
resources-lede = FHIR-Ressourcen durchsuchen, suchen, erstellen und bearbeiten. In natÃ¼rlicher Sprache suchen oder die Abfrage selbst bauen, dann ein Ergebnis zum Bearbeiten Ã¶ffnen.
resources-create = Neu erstellen
resources-save-blocked = Beheben Sie die Validierungsprobleme vor dem Speichern.
resources-save-invalid = Das JSON ist ungÃ¼ltig â€” beheben Sie es vor dem Speichern.
resources-edit-title = Ressource bearbeiten
resources-tab-edit = Bearbeiten
resources-tab-history = Verlauf
resources-types-heading = Ressourcentypen

queries-saved-group = Gespeichert

nav-collapse = MenÃ¼ einklappen

batch-heading = Batch / Transaction
batch-lede = Lade ein FHIR-Bundle hoch, prÃ¼fe die auszufÃ¼hrenden Aktionen, fÃ¼hre es gegen diesen Server aus und lies das Ergebnis jedes Eintrags.
batch-upload = Hochladen
batch-drop-hint = Bundle-JSON-Datei hier ablegen
batch-drop-browse = oder klicken zum Durchsuchen
batch-invalid-json = Diese Datei ist kein gÃ¼ltiges JSON
batch-not-a-bundle = Dieses JSON ist kein FHIR-Bundle
batch-bad-type = Hier lassen sich nur Bundles vom Typ batch oder transaction ausfÃ¼hren
batch-request = Anfrage
batch-entries = EintrÃ¤ge
batch-semantics-batch = Batch: EintrÃ¤ge laufen unabhÃ¤ngig â€” ein fehlgeschlagener Eintrag stoppt die anderen nicht und macht sie nicht rÃ¼ckgÃ¤ngig.
batch-semantics-transaction = Transaction: alles oder nichts â€” schlÃ¤gt ein Eintrag fehl, rollt der Server das gesamte Bundle zurÃ¼ck.
batch-tab-actions = Aktionen
batch-tab-json = Bundle-JSON
batch-no-body = (kein Body â€” dieser Eintrag adressiert nur eine Ressource)
batch-cancel = Abbrechen
batch-upload-another = Weitere hochladen
batch-execute = AusfÃ¼hren
batch-response-heading = Ergebnisse pro Aktion
batch-sum-created = erstellt
batch-sum-updated = aktualisiert
batch-sum-other = gelesen/sonstige
batch-sum-failed = fehlgeschlagen
batch-request-failed = Die Anfrage ist fehlgeschlagen
batch-back = ZurÃ¼ck zum Bundle
batch-execute-again = Erneut ausfÃ¼hren

## Bulk Import workspace (#527)

bulk-import-title = Massenimport
bulk-import-new = Neue Submission
bulk-import-create-title = Bulk Submission anlegen
bulk-import-field-name = Name der Submission
bulk-import-field-recipient = Basis-URL des EmpfÃ¤ngers
bulk-import-field-recipient-hint = Die Basis-URL des Servers, an den die Daten Ã¼bermittelt werden.
bulk-import-auth = Authentifizierung
bulk-import-auth-hint = Wie gegenÃ¼ber dem EmpfÃ¤ngerserver authentifiziert wird.
bulk-import-auth-none = Keine
bulk-import-auth-none-hint = Es wird kein Authorization-Header gesendet.
bulk-import-auth-backend = Backend-Services-Authentifizierung
bulk-import-auth-backend-hint = Holt ein Zugriffstoken und sendet es als Bearer im Authorization-Header.
bulk-import-field-client-id = Client-ID
bulk-import-field-client-id-hint = Registrieren Sie diesen Datenanbieter beim EmpfÃ¤nger und erhalten Sie eine Client-ID.
bulk-import-field-token-url = Token-URL
bulk-import-field-token-url-hint = Token-Endpunkt-URL des Autorisierungsservers.
bulk-import-jwks-hint = Registrieren Sie den Ã¶ffentlichen SchlÃ¼ssel dieses Servers beim EmpfÃ¤nger Ã¼ber die JWKS-URL:
bulk-import-test-auth = Authentifizierung testen
bulk-import-test-auth-ok = Authentifizierung erfolgreich.
bulk-import-create-submit = Submission anlegen
bulk-import-unavailable = Das Storage-Backend hostet keinen Settings-Store; Submissions kÃ¶nnen nicht gespeichert werden.
bulk-import-submissions = Submissions
bulk-import-records = EintrÃ¤ge
bulk-import-col-name = Name
bulk-import-col-status = Status
bulk-import-col-created = Erstellt
bulk-import-col-manifests = Manifeste
bulk-import-col-destination = Ziel
bulk-import-empty = Noch keine Submissions. Legen Sie eine an, um zu beginnen.
bulk-import-all = Alle Submissions
bulk-import-status-not-started = Nicht gestartet
bulk-import-status-in-progress = In Bearbeitung
bulk-import-status-stopped = Angehalten
bulk-import-status-completed = Abgeschlossen
bulk-import-detail-recipient = DatenempfÃ¤nger
bulk-import-detail-id = Submission-ID
bulk-import-detail-submitter = Einreicher
bulk-import-detail-created = Erstellt
bulk-import-detail-status = Status
bulk-import-detail-auth = Authentifizierung
bulk-import-abort = Abbrechen
bulk-import-complete = AbschlieÃŸen
bulk-import-delete = LÃ¶schen
bulk-import-add-manifest = Manifest hinzufÃ¼gen
bulk-import-add-manifest-title = Manifest hinzufÃ¼gen
bulk-import-add-manifest-submit = HinzufÃ¼gen
bulk-import-field-manifest-url = Manifest-URL
bulk-import-field-manifest-url-hint = URL eines Bulk-Export-Manifests mit einem vorkoordinierten FHIR-Datensatz.
bulk-import-field-fhir-base = FHIR-Basis-URL
bulk-import-field-fhir-base-hint = Basis-URL, die der EmpfÃ¤nger beim AuflÃ¶sen relativer Referenzen verwendet. Leer lassen, um die Basis-URL des Manifests zu verwenden.
bulk-import-field-output-format = Ausgabeformat
bulk-import-field-output-format-hint = Das Format der Bulk-Data-Dateien im Manifest.
bulk-import-field-headers = Header fÃ¼r Dateiabrufe
bulk-import-field-headers-hint = HTTP-Header, die der EmpfÃ¤nger beim Abruf einer Datendatei verwenden soll, je Zeile "Name: Wert".
bulk-import-manifests = Manifeste
bulk-import-no-manifests = Noch keine Manifeste. FÃ¼gen Sie eines hinzu, um Daten zu Ã¼bermitteln.
bulk-import-submit = Ãœbermitteln
bulk-import-submit-all = Alle Ã¼bermitteln
bulk-import-remove = Entfernen
bulk-import-log = Submission-Protokoll
bulk-import-log-empty = Noch nichts Ã¼bermittelt.
bulk-import-field-submitter-system = Einreicher-System
bulk-import-field-submitter-value = Einreicher-Wert
bulk-import-field-submitter-hint = Muss einem beim EmpfÃ¤nger registrierten Identifier entsprechen (auÃŸerhalb des Protokolls abgestimmt). Leer lassen fÃ¼r die generierten Standardwerte.
bulk-import-field-submission-id = Submission-ID
bulk-import-field-submission-id-hint = Eindeutig je Einreicher. Leer lassen, um eine UUID zu generieren.
bulk-import-processing = Verarbeitung
bulk-import-processing-waiting = Warten auf den ersten Statusbericht des EmpfÃ¤ngers â€¦
bulk-import-result = Ergebnis
bulk-import-result-finished = Verarbeitung abgeschlossen um
bulk-import-result-outputs = Ausgabedateien
bulk-import-result-errors = Fehlerdateien
bulk-import-abort-manifest = Abbrechen

## Administrative HTS-UI (crates/hts-ui) â€” Phase-1-Stubs
##
## Der vollstÃ¤ndige Katalog wird in Phase 1.4 / Phase 2 ergÃ¤nzt, entsprechend
## `edson/docs/hts-ui-design.md` Â§7. Diese Stubs decken Base-Layout,
## Seitennavigation und den Dashboard-Platzhalter der Phase-1-Blocker-Slice ab.
## Sie mÃ¼ssen paritÃ¤tisch zu en/es/main.ftl bleiben.

-hts-app-name = Helios Terminologieserver
hts-app-title = { -hts-app-name }

hts-nav-section-work = Terminologie
hts-nav-section-tools = Werkzeuge
hts-nav-section-server = Server
hts-nav-dashboard = Ãœbersicht
hts-nav-code-systems = Codesysteme
hts-nav-value-sets = Wertemengen
hts-nav-concept-maps = Konzeptzuordnungen
hts-nav-operations = Operationen
hts-nav-import = Import
hts-nav-diagnostics = Diagnose

hts-fhir-version-heading = FHIR-Version
hts-fhir-version = FHIR { $version }

hts-dashboard-title = Ãœbersicht
hts-dashboard-subtitle = Zustand des Terminologieservers, Katalogbestand und Schnellaktionen.

## Dashboard-Zeilen (visuell verborgen, nur fÃ¼r Screenreader).

hts-dashboard-row-status = Serverstatus
hts-dashboard-row-inventory = Geladener Bestand
hts-dashboard-row-metrics = Verkehrsmetriken
hts-dashboard-quick-links = Schnelllinks

## Dashboard-Kacheln.

hts-dashboard-tile-status = Status
hts-dashboard-tile-backend = Backend
hts-dashboard-tile-uptime = Laufzeit
hts-dashboard-tile-fhir-version = FHIR-Version
hts-dashboard-tile-loaded-systems = Geladene Systeme
hts-dashboard-tile-loaded-systems-hint = Aus TerminologyCapabilities.codeSystem[]
hts-dashboard-tile-bundled-data = GebÃ¼ndelte Daten
hts-dashboard-tile-bundled-data-value = { $mib } MiB
hts-dashboard-tile-bundled-data-hint = Aus dem HTS_BOOTSTRAP_DIR-Umfang
hts-dashboard-tile-requests = Anfragen
hts-dashboard-tile-avg-latency = Durchschn. Latenz
hts-dashboard-tile-metrics-hint = Aus /metrics â€” Wave 2

## `status`-Werte aus /health, per SchlÃ¼ssel Ã¼bersetzbar.

hts-dashboard-status-ok = OK

## Degradiert-Banner (Design-Dokument Â§7-Kontrakt).

hts-degraded-title = Das Terminologie-Backend ist nicht vollstÃ¤ndig verfÃ¼gbar
hts-degraded-body = Einige Kacheln werden ausgeblendet, bis HTS wieder erreichbar ist. Interaktive Bedienelemente sind auf betroffenen Seiten deaktiviert.
hts-degraded-reason-client-build = Der ausgehende HTTP-Client konnte nicht erstellt werden.
hts-degraded-reason-upstream-down = Der Terminologieserver ist nicht erreichbar.
hts-degraded-reason-upstream-timeout = Der Terminologieserver hat nicht rechtzeitig geantwortet.
hts-degraded-reason-upstream-error = Der Terminologieserver hat einen Fehlerstatus zurÃ¼ckgegeben.
hts-degraded-reason-upstream-shape = Der Terminologieserver hat eine Antwort in unerwarteter Form zurÃ¼ckgegeben.
hts-degraded-reason-bootstrapping = Der Terminologieserver lÃ¤dt noch seine Ausgangsdaten.
hts-degraded-reason-unknown = Der Terminologieserver ist vorÃ¼bergehend nicht verfÃ¼gbar.

## Dialekt-Chip (Topbar, sitzungsweiter displayLanguage / Accept-Language â€” Â§7.1).

hts-dialect-label = Dialekt
hts-dialect-prefix = Dialekt:
hts-dialect-heading = Sitzungsdialekt
hts-dialect-hint = Steuert displayLanguage bei Expansionen und Accept-Language bei LesevorgÃ¤ngen. Feldwerte auf der Operationen-Seite haben Vorrang.

## OperationOutcome-Partial (gemeinsam â€” Â§7 / Â§11).

hts-outcome-severity = Schweregrad: { $severity }
hts-outcome-request-id = Anfrage-ID: { $id }
hts-outcome-code-not-found = Die angeforderte Ressource wurde nicht gefunden.
hts-outcome-code-invalid = Die Anfrage wurde als ungÃ¼ltig zurÃ¼ckgewiesen.
hts-outcome-code-too-costly = Die angeforderte Operation wurde als zu teuer zurÃ¼ckgewiesen.
hts-outcome-code-unknown = Der Server hat ein Problem gemeldet, das die UI nicht kennt.
hts-degraded-since = Seit { $timestamp }

## HTS Slice B â€” CodeSystem-Browser + Detailansicht mit eingebettetem Workbench
## (Design-Dokument Â§7.2 + Â§7.3). Jeder SchlÃ¼ssel hat ein Pendant in en/es/main.ftl.

## CodeSystem-Statuspillen (Browser-Zeilen und Detail-Kopfzeile).

hts-cs-status-draft = Entwurf
hts-cs-status-active = aktiv
hts-cs-status-retired = zurÃ¼ckgezogen
hts-cs-status-unknown = unbekannt

## CodeSystem-Browserseite.

hts-cs-browser-title = Codesysteme
hts-cs-browser-subtitle = Durchsuche den CodeSystem-Katalog des Terminologieservers und Ã¶ffne eine Zeile, um Metadaten und Workbench einzusehen.
hts-cs-browser-filter-legend = CodeSysteme filtern
hts-cs-browser-filter-url = Kanonische URL
hts-cs-browser-filter-version = Version
hts-cs-browser-filter-name = Name
hts-cs-browser-filter-title = Titel
hts-cs-browser-filter-status = Status
hts-cs-browser-filter-search = Suchen
hts-cs-browser-filter-reset = ZurÃ¼cksetzen
hts-cs-browser-empty = Keine CodeSysteme entsprechen diesen Filtern.
hts-cs-browser-load-more = Mehr laden
hts-cs-browser-showing-count = Es werden { $count ->
    [one] { $count } CodeSystem angezeigt
   *[other] { $count } CodeSysteme angezeigt
}
hts-cs-browser-table-caption = CodeSysteme, die zu den aktiven Filtern passen.
hts-cs-browser-column-url = URL
hts-cs-browser-column-version = Version
hts-cs-browser-column-title = Titel
hts-cs-browser-column-status = Status
hts-cs-browser-error-title = CodeSysteme konnten nicht aufgelistet werden

## CodeSystem-Detailseite.

hts-cs-detail-title = { $name } Â· CodeSystem
hts-cs-detail-title-fallback = CodeSystem
hts-cs-detail-eyebrow = CodeSystem
hts-cs-detail-section-identity = IdentitÃ¤t
hts-cs-detail-section-content = Inhalt
hts-cs-detail-content-mode = Inhaltsmodus
hts-cs-detail-count = Anzahl Konzepte
hts-cs-detail-publisher = Herausgeber
hts-cs-detail-jurisdiction = ZustÃ¤ndigkeit
hts-cs-detail-supersedes = Ersetzt
hts-cs-detail-superseded-by = Ersetzt durch
hts-cs-detail-tabs-label = CodeSystem-Workbench-Abschnitte
hts-cs-detail-tab-metadata = Metadaten
hts-cs-detail-tab-lookup = Nachschlagen
hts-cs-detail-tab-validate = Validieren
hts-cs-detail-tab-subsumes = Subsumption
hts-cs-detail-workbench-hint = WÃ¤hle eine Operation, die gegen dieses CodeSystem laufen soll.
hts-cs-detail-result-empty = FÃ¼hre die Operation aus, um das Ergebnis hier zu sehen.

## $lookup-Formular + Ergebnisbeschriftungen.

hts-cs-lookup-heading = Konzept nachschlagen
hts-cs-lookup-code = Code
hts-cs-lookup-version = Version
hts-cs-lookup-display-language = Anzeigesprache
hts-cs-lookup-display-language-placeholder = z. B. de-DE
hts-cs-lookup-properties-legend = Eigenschaften
hts-cs-lookup-designations = Bezeichnungen
hts-cs-lookup-properties = Eigenschaften
hts-cs-lookup-no-match = HTS hat kein passendes Konzept zurÃ¼ckgegeben.

## $validate-code-Formular + Ergebnisbeschriftungen.

hts-cs-validate-heading = Code validieren
hts-cs-validate-mode-legend = Eingabemodus
hts-cs-validate-mode-code = Einzelcode
hts-cs-validate-mode-coding = Coding
hts-cs-validate-code = Code
hts-cs-validate-display = Anzeige
hts-cs-validate-coding-legend = Coding
hts-cs-validate-coding-system = System
hts-cs-validate-coding-code = Code
hts-cs-validate-coding-display = Anzeige
hts-cs-validate-badge-true = gÃ¼ltig
hts-cs-validate-badge-false = ungÃ¼ltig
hts-cs-validate-message = Meldung

## $subsumes-Formular + Ergebnisbeschriftungen.

hts-cs-subsumes-heading = Subsumption prÃ¼fen
hts-cs-subsumes-scoped-system = System (festgelegt)
hts-cs-subsumes-code-a = Code A
hts-cs-subsumes-code-b = Code B
hts-cs-subsumes-outcome-equivalent = Die Codes sind Ã¤quivalent.
hts-cs-subsumes-outcome-subsumes = Code A subsumiert Code B.
hts-cs-subsumes-outcome-subsumed-by = Code A wird von Code B subsumiert.
hts-cs-subsumes-outcome-not-subsumed = Keiner der Codes subsumiert den anderen.

## Geteilte Workbench-Chrome (auch fÃ¼r Slice C/D/E).

hts-workbench-run = AusfÃ¼hren
hts-workbench-raw-response = Rohanfrage und -antwort
hts-workbench-copy-url = Anfrage-URL
hts-workbench-format-json = JSON
hts-workbench-format-xml = XML

## ZusÃ¤tzlicher Degradiert-Grund fÃ¼r 404 beim CodeSystem-Read (Â§7.3).

hts-degraded-reason-upstream-not-found = Der Terminologieserver hat diese Ressource nicht gefunden.

## HTS Slice C â€” ValueSet-Browser + Detailseite mit $expand-Werkbank
## (design doc Â§7.4 + Â§7.4.1). Jeder SchlÃ¼ssel hat ein Pendant in en/es/main.ftl.

## Statusabzeichen fÃ¼r ValueSet.

hts-vs-status-draft = Entwurf
hts-vs-status-active = aktiv
hts-vs-status-retired = zurÃ¼ckgezogen
hts-vs-status-unknown = unbekannt

## VS-Browser-Seite.

hts-vs-browser-title = ValueSets
hts-vs-browser-subtitle = Durchsuche den ValueSet-Katalog des Terminologieservers und Ã¶ffne eine Zeile, um Metadaten oder eine Expansion einzusehen.
hts-vs-browser-filter-legend = ValueSets filtern
hts-vs-browser-filter-url = Kanonische URL
hts-vs-browser-filter-version = Version
hts-vs-browser-filter-name = Name
hts-vs-browser-filter-title = Titel
hts-vs-browser-filter-status = Status
hts-vs-browser-filter-search = Suchen
hts-vs-browser-filter-reset = ZurÃ¼cksetzen
hts-vs-browser-empty = Keine ValueSets fÃ¼r diese Filter.
hts-vs-browser-load-more = Mehr laden
hts-vs-browser-showing-count = Zeige { $count ->
    [one] { $count } ValueSet
   *[other] { $count } ValueSets
}
hts-vs-browser-table-caption = ValueSets, die den aktiven Filtern entsprechen.
hts-vs-browser-column-url = URL
hts-vs-browser-column-version = Version
hts-vs-browser-column-title = Titel
hts-vs-browser-column-status = Status

## VS-Detailseite.

hts-vs-detail-title = { $name } Â· ValueSet
hts-vs-detail-title-fallback = ValueSet
hts-vs-detail-eyebrow = ValueSet
hts-vs-detail-section-identity = IdentitÃ¤t
hts-vs-detail-section-governance = Verwaltung
hts-vs-detail-publisher = Herausgeber
hts-vs-detail-jurisdiction = ZustÃ¤ndigkeit
hts-vs-detail-immutable = UnverÃ¤nderlich
hts-vs-detail-immutable-yes = ja
hts-vs-detail-immutable-no = nein
hts-vs-detail-purpose = Zweck
hts-vs-detail-copyright = Urheberrecht
hts-vs-detail-tabs-label = ValueSet-Werkbank-Abschnitte
hts-vs-detail-tab-metadata = Metadaten
hts-vs-detail-tab-expand = Expandieren
hts-vs-detail-workbench-hint = WÃ¤hle eine Operation, die auf diesem ValueSet laufen soll.
hts-vs-detail-result-empty = FÃ¼hre die Operation aus, um das Ergebnis hier zu sehen.

## $expand â€” Formular und Ergebnisse.

hts-vs-expand-heading = Diesen ValueSet expandieren
hts-vs-expand-scoped-valueset = ValueSet (fixiert)
hts-vs-expand-filter = Filter
hts-vs-expand-filter-placeholder = Code oder Anzeigetext
hts-vs-expand-count = count
hts-vs-expand-offset = offset
hts-vs-expand-display-language = Anzeigesprache
hts-vs-expand-display-language-placeholder = z. B. de-DE
hts-vs-expand-flags-legend = Optionen
hts-vs-expand-active-only = Nur aktive Konzepte
hts-vs-expand-include-designations = Designationen einschlieÃŸen
hts-vs-expand-mode-legend = Ergebnisformat
hts-vs-expand-mode-flat = Flach
hts-vs-expand-mode-tree = Baum
hts-vs-expand-use-supplement-legend = ErgÃ¤nzungen anwenden
hts-vs-expand-use-supplement-placeholder = Kanonische URL
hts-vs-expand-advanced-summary = Erweitert
hts-vs-expand-date = Datum
hts-vs-expand-date-placeholder = ISO 8601 (z. B. 2025-06-01)
hts-vs-expand-property-legend = Eigenschaften
hts-vs-expand-property-placeholder = Eigenschaftscode
hts-vs-expand-tx-resource-legend = tx-resource
hts-vs-expand-tx-resource-placeholder = Kanonische URL oder Referenz
hts-vs-expand-system-version-legend = system-version
hts-vs-expand-system-version-placeholder = System|Version
hts-vs-expand-check-system-version-legend = check-system-version
hts-vs-expand-force-system-version-legend = force-system-version
hts-vs-expand-default-valueset-version = default-valueset-version
hts-vs-expand-threshold = Too-costly-Schwelle
hts-vs-expand-ceiling-tooltip = UI-Obergrenze: { $ceiling } (hÃ¶here Werte werden verworfen)
hts-vs-expand-ceiling-note = Obergrenze: { $ceiling }
hts-vs-expand-ceiling-warning-title = Schwelle Ã¼ber der UI-Obergrenze
hts-vs-expand-ceiling-warning-body = Schwelle { $requested } liegt Ã¼ber der UI-Obergrenze â€” der Header wurde nicht angehÃ¤ngt.
hts-vs-expand-ceiling-value = Obergrenze: { $ceiling }
hts-vs-expand-too-costly-title = Expansion als zu teuer abgelehnt
hts-vs-expand-too-costly-body = HTS hat die Expansion oberhalb der aktuellen Schwelle abgelehnt. HÃ¶her setzen und erneut versuchen, oder den Filter enger fassen.
hts-vs-expand-raise-threshold = Schwelle anheben auf
hts-vs-expand-raise-submit = Erneut versuchen
hts-vs-expand-tree-label = zeige den ganzen Baum { $count ->
    [one] { $count } Blatt
   *[other] { $count } BlÃ¤tter
}
hts-vs-expand-total-label = insgesamt { $total }
hts-vs-expand-total-unknown = insgesamt (unbekannt)
hts-vs-expand-offset-label = offset { $offset }
hts-vs-expand-filter-no-match = Kein Element entspricht dem Filter "{ $filter }".
hts-vs-expand-no-members = Diese Expansion enthÃ¤lt keine Elemente.
hts-vs-expand-column-code = Code
hts-vs-expand-column-display = Anzeige
hts-vs-expand-column-system = System
hts-vs-expand-load-more = Mehr laden
hts-vs-expand-echoed-parameters = Echo-Parameter

## HTS Slice D â€” ConceptMap-Browser und Detail mit eingebettetem
## $translate-Workbench (Designdokument Â§7.5). Jeder SchlÃ¼ssel hat ein
## Pendant in en/es/main.ftl.

## ConceptMap-Status-Pillen.

hts-cm-status-draft = Entwurf
hts-cm-status-active = aktiv
hts-cm-status-retired = ausgemustert
hts-cm-status-unknown = unbekannt

## CM-Browser-Seite.

hts-cm-browser-title = ConceptMaps
hts-cm-browser-subtitle = Durchsuche den Katalog der ConceptMaps auf dem Terminologieserver und Ã¶ffne eine Zeile, um Metadaten anzuzeigen oder eine Ãœbersetzung auszufÃ¼hren.
hts-cm-browser-filter-legend = ConceptMaps filtern
hts-cm-browser-filter-url = Kanonische URL
hts-cm-browser-filter-name = Name
hts-cm-browser-filter-title = Titel
hts-cm-browser-filter-source = Quellsystem
hts-cm-browser-filter-target = Zielsystem
hts-cm-browser-filter-status = Status
hts-cm-browser-filter-search = Suchen
hts-cm-browser-filter-reset = ZurÃ¼cksetzen
hts-cm-browser-empty = Keine ConceptMaps entsprechen diesen Filtern.
hts-cm-browser-load-more = Mehr laden
hts-cm-browser-showing-count = { $count ->
    [one] { $count } ConceptMap wird angezeigt
   *[other] { $count } ConceptMaps werden angezeigt
}
hts-cm-browser-table-caption = ConceptMaps, die den aktiven Filtern entsprechen.
hts-cm-browser-column-url = URL
hts-cm-browser-column-version = Version
hts-cm-browser-column-title = Titel
hts-cm-browser-column-status = Status

## CM-Detailseite.

hts-cm-detail-title = { $name } Â· ConceptMap
hts-cm-detail-title-fallback = ConceptMap
hts-cm-detail-eyebrow = ConceptMap
hts-cm-detail-section-identity = IdentitÃ¤t
hts-cm-detail-section-mapping = Mapping
hts-cm-detail-publisher = Herausgeber
hts-cm-detail-jurisdiction = ZustÃ¤ndigkeit
hts-cm-detail-purpose = Zweck
hts-cm-detail-source-uri = Quelle
hts-cm-detail-target-uri = Ziel
hts-cm-detail-group-count = Gruppen
hts-cm-detail-tabs-label = Workbench-Bereiche der ConceptMap
hts-cm-detail-tab-metadata = Metadaten
hts-cm-detail-tab-translate = Ãœbersetzen
hts-cm-detail-workbench-hint = WÃ¤hle eine Operation, um sie fÃ¼r diese ConceptMap auszufÃ¼hren.
hts-cm-detail-result-empty = FÃ¼hre die Operation aus, um das Ergebnis hier zu sehen.

## $translate-Formular und -Ergebnisse.

hts-cm-translate-heading = Einen Code Ã¼bersetzen
hts-cm-translate-scoped-map = ConceptMap (fest)
hts-cm-translate-direction-legend = Richtung
hts-cm-translate-direction-forward = VorwÃ¤rts
hts-cm-translate-direction-reverse = RÃ¼ckwÃ¤rts
hts-cm-translate-source-legend = Quellcodierung
hts-cm-translate-source-system = System
hts-cm-translate-source-system-placeholder = kanonische URL
hts-cm-translate-source-code = Code
hts-cm-translate-source-display = Anzeige
hts-cm-translate-source-display-placeholder = optional
hts-cm-translate-reverse-legend = RÃ¼ckwÃ¤rts-Quelle
hts-cm-translate-target-code = Zielcode
hts-cm-translate-target-code-hint = Im RÃ¼ckwÃ¤rtsmodus erforderlich.
hts-cm-translate-target-legend = Ziel-EinschrÃ¤nkungen
hts-cm-translate-target-system = Zielsystem
hts-cm-translate-target-system-placeholder = kanonische URL
hts-cm-translate-source-url = Quell-ValueSet
hts-cm-translate-source-url-placeholder = kanonische URL (optional)
hts-cm-translate-target-url = Ziel-ValueSet
hts-cm-translate-target-url-placeholder = kanonische URL (optional)
hts-cm-translate-date = Datum
hts-cm-translate-date-placeholder = ISO 8601 (z. B. 2025-06-01)
hts-cm-translate-submit = Ãœbersetzen
hts-cm-translate-matches-heading = Treffer
hts-cm-translate-matches-count = { $count ->
    [one] { $count } Treffer
   *[other] { $count } Treffer
}
hts-cm-translate-no-matches = Keine Treffer fÃ¼r diese Quelle.
hts-cm-translate-column-code = Code
hts-cm-translate-column-system = System
hts-cm-translate-column-display = Anzeige
hts-cm-translate-column-mapping = { $kind ->
    [equivalence] Ã„quivalenz
    [relationship] Beziehung
   *[other] Mapping
}
hts-cm-translate-column-origin = Ursprung
hts-cm-translate-column-mapping-equivalence = Ã„quivalenz
hts-cm-translate-column-mapping-relationship = Beziehung
hts-cm-translate-validate-forward-missing = VorwÃ¤rtsÃ¼bersetzung benÃ¶tigt sowohl `code` als auch `system`.
hts-cm-translate-validate-reverse-missing-target-code = RÃ¼ckwÃ¤rtsÃ¼bersetzung benÃ¶tigt `targetCode`.

## HTS Slice E -- Standalone-Operations-Workbench (design doc s7.6).

hts-operations-title = Operationen-Workbench
hts-operations-eyebrow = Terminologie
hts-operations-subtitle = Terminologieoperationen gegen den verbundenen Server ausfuehren. Jede Operation geht als POST, unabhaengig vom Verb des Formulars.
hts-operations-selector-label = Operation
hts-operations-resource-tabs-label = Ressourcenfamilie
hts-operations-resource-code-system = CodeSystem
hts-operations-resource-value-set = ValueSet
hts-operations-result-empty = Fuehre die Operation aus, um das Ergebnis hier zu sehen.
hts-operations-scope-legend = Bereich
hts-operations-scope-system = Kanonische URL des CodeSystems
hts-operations-scope-instance = Instanz-ID
hts-operations-scope-instance-placeholder = Instanz-ID
hts-operations-scope-canonical = Kanonische URL
hts-operations-not-implemented = Diese Operation kommt in Slice E2.
hts-operations-closure-stateless-warning = Der Closure-Zustand lebt auf dem Server unter dem angegebenen `name`. Die UI speichert ihn nicht zwischen Anfragen.
hts-operations-closure-empty-graph = Noch keine Kanten -- sende mindestens ein Coding, um Knoten hinzuzufuegen.

hts-operations-op-lookup = $lookup
hts-operations-op-validate-code = $validate-code
hts-operations-op-subsumes = $subsumes
hts-operations-op-expand = $expand
hts-operations-op-translate = $translate
hts-operations-op-closure = $closure
hts-operations-op-batch-validate = batch-validate

hts-cs-lookup-useSupplement = Ergaenzung
hts-cs-lookup-useSupplement-hint = Optionale kanonische URL einer CodeSystem-Ergaenzung, die ueber das Basis-System gelegt wird.
hts-cs-lookup-result-heading = Lookup-Ergebnis
hts-cs-lookup-fact-name = Name
hts-cs-lookup-fact-version = Version
hts-cs-lookup-fact-display = Anzeige
hts-cs-lookup-fact-definition = Definition

hts-cs-validate-version = CodeSystem-Version
hts-cs-validate-systemVersion = System-Version ueberschreiben
hts-cs-validate-mode-CodeableConcept = CodeableConcept
hts-cs-validate-displayLanguage = Anzeigesprache
hts-cs-validate-advanced = Erweiterte Parameter
hts-cs-validate-date = Datum
hts-cs-validate-activeOnly = Nur aktive Codes
hts-cs-validate-abstract = Abstrakte Codes zulassen
hts-cs-validate-lenient-display-validation = Weiche Display-Validierung
hts-cs-validate-useSupplement = Ergaenzungs-URL
hts-cs-validate-system-version = System-Version fixieren
hts-cs-validate-check-system-version = System-Version pruefen
hts-cs-validate-force-system-version = System-Version erzwingen
hts-cs-validate-result-heading = Validate-Ergebnis
hts-cs-validate-result-badge-true = Gueltig
hts-cs-validate-result-badge-false = Nicht gueltig
hts-cs-validate-fact-code = Code
hts-cs-validate-fact-display = Anzeige
hts-cs-validate-fact-message = Nachricht

hts-cs-subsumes-version = Version
hts-cs-subsumes-codeA = Code A
hts-cs-subsumes-codeB = Code B
hts-cs-subsumes-result-heading = Subsumtionsergebnis

hts-vs-expand-displayLanguage = Anzeigesprache
hts-vs-expand-activeOnly = Nur aktive
hts-vs-expand-includeDesignations = Designationen einbeziehen
hts-vs-expand-designation = Designation
hts-vs-expand-designation-hint = Chip-Filter -- ein `use|value`-Paar pro Zeile (wiederholbar).
hts-vs-expand-advanced = Erweiterte Parameter
hts-vs-expand-threshold-hint = HTS lehnt Expansionen ueber dem Schwellenwert ab. UI-Obergrenze: { $ceiling }.
hts-vs-expand-result-heading = Expansion
hts-vs-expand-total = Gesamt { $n }
hts-vs-expand-count-shown = angezeigt { $n }

hts-vs-validate-heading = Code gegen ein ValueSet validieren
hts-vs-validate-source-legend = ValueSet-Quelle
hts-vs-validate-source-canonical = Kanonische URL
hts-vs-validate-source-instance = Instanz-ID
hts-vs-validate-source-inline = Inline-JSON
hts-vs-validate-mode-legend = Eingabeform
hts-vs-validate-mode-code = Code
hts-vs-validate-mode-coding = Coding
hts-vs-validate-mode-CodeableConcept = CodeableConcept
hts-vs-validate-code = Code
hts-vs-validate-system = System
hts-vs-validate-systemVersion = System-Version
hts-vs-validate-display = Anzeige
hts-vs-validate-coding-legend = Coding
hts-vs-validate-coding-system = System
hts-vs-validate-coding-code = Code
hts-vs-validate-coding-display = Display
hts-vs-validate-displayLanguage = Anzeigesprache
hts-vs-validate-valueSetVersion = ValueSet-Version
hts-vs-validate-advanced = Erweiterte Parameter
hts-vs-validate-date = Datum
hts-vs-validate-activeOnly = Nur aktive
hts-vs-validate-abstract = Abstrakte Codes zulassen
hts-vs-validate-lenient-display-validation = Weiche Display-Validierung
hts-vs-validate-useSupplement = Ergaenzungs-URL
hts-vs-validate-tx-resource = Extra tx-resource
hts-vs-validate-default-valueset-version = Standard-ValueSet-Version
hts-vs-validate-no-membership = Der Code ist kein Mitglied des ValueSets.
hts-vs-validate-result-heading = Validate-Ergebnis
hts-vs-validate-result-badge-true = Gueltig
hts-vs-validate-result-badge-false = Nicht gueltig
hts-vs-validate-fact-code = Code
hts-vs-validate-fact-system = System
hts-vs-validate-fact-display = Anzeige
hts-vs-validate-fact-message = Nachricht

hts-cm-translate-code = Code
hts-cm-translate-system = System
hts-cm-translate-display = Anzeige
hts-cm-translate-targetCode = Ziel-Code
hts-cm-translate-targetSystem = Ziel-System
hts-cm-translate-result-heading = Translate-Ergebnis
hts-cm-translate-result-badge-true = Uebereinstimmung
hts-cm-translate-result-badge-false = Keine Uebereinstimmung

hts-cm-closure-heading = Closure-Graph
hts-cm-closure-name = Closure-Name
hts-cm-closure-name-hint = Vom Client vergebener Name, der die Closure-Tabelle auf dem Server ueber Anfragen hinweg identifiziert.
hts-cm-closure-concepts-legend = Konzepte
hts-cm-closure-concepts-hint = Bis zu drei Ausgangs-Codings hinzufuegen; jede Zeile ist ein System-Code-Paar.
hts-cm-closure-concept-system = System
hts-cm-closure-concept-code = Code
hts-cm-closure-result-heading = Closure-Kanten
hts-cm-closure-edge-source = Quelle
hts-cm-closure-edge-equivalence = Aequivalenz
hts-cm-closure-edge-target = Ziel

hts-vs-batch-heading = Codes gegen ein ValueSet als Batch validieren
hts-vs-batch-target-value-set-label = Ziel-ValueSet
hts-vs-batch-rows-legend = Zeilen
hts-vs-batch-rows-hint = Einen Code pro Zeile eingeben; leere Zeilen werden verworfen.
hts-vs-batch-row-code = Code
hts-vs-batch-row-system = System
hts-vs-batch-row-display = Anzeige
hts-vs-batch-row-timeout = Zeitueberschreitung
hts-vs-batch-row-placeholder = --
hts-vs-batch-result-heading = Batch-Ergebnis
hts-vs-batch-target-hint = Ziel-ValueSet: { $target }
hts-vs-batch-column-code = Code
hts-vs-batch-column-system = System
hts-vs-batch-column-display = Anzeige
hts-vs-batch-column-result = Ergebnis
hts-vs-batch-progress = { $n } von { $m } abgeschlossen
hts-vs-batch-progress-final = { $m } abgeschlossen
