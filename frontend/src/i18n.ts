/**
 * Chess app i18n: English plus Spanish, French, and German.
 *
 * Mirrors captive-portal/src/i18n.ts and shares its localStorage key
 * ("mossymesh-portal-lang") so the language picked on the landing page
 * carries into the app. The two vite apps build independently, so the
 * plumbing is duplicated here on purpose; the storage key is the contract.
 */

export type AppLang = "en" | "es" | "fr" | "de";

export const APP_LANGS: AppLang[] = ["en", "es", "fr", "de"];

const LANG_KEY = "mossymesh-portal-lang";

export interface AppStrings {
  app: {
    offlineNotice: string;
    notFound: string;
    badge: string;
    subtitle: string;
    portalCopy: string;
  };
  chess: {
    checkmateWhiteWins: string;
    checkmateBlackWins: string;
    stalemate: string;
    threefold: string;
    insufficientMaterial: string;
    draw: string;
    whiteInCheck: string;
    blackInCheck: string;
    whiteToMove: string;
    blackToMove: string;
    whiteToMoveReady: string;
    illegalMove: string;
    noteLocalOnly: string;
    noteUndone: string;
    notePlayOffline: string;
    noteSeeking: string;
    noteQueuedLora: string;
    noteAppliedLocal: string;
    noteSubmitting: string;
    noteConfirmed: string;
    noteRejected: string;
    noteUnreachable: string;
    noteNewGame: string;
    moveHistory: string;
    noMoves: string;
    playOffline: string;
    seekPeer: string;
    undo: string;
    newGame: string;
    evalDaemonTitle: string;
    evalLocalTitle: string;
    evalLoading: string;
    evalAria: (label: string) => string;
    evalText: (score: string) => string;
  };
  visualizer: {
    title: string;
    scanning: string;
    summary: (islands: number, peers: number, name: string) => string;
    probing: string;
    nodes: (n: number) => string;
    ping: (latency: number, hops: number) => string;
    signal: (rssi: number) => string;
    signalAria: (rssi: number, bars: number) => string;
    hint: string;
  };
  daemonToast: {
    message: string;
    dismiss: string;
  };
  ble: {
    unavailable: string;
    enable: string;
    scanning: string;
    on: string;
    failed: string;
    useAsRelay: string;
    relayingVia: (peer: string, frames: number) => string;
  };
}

const en: AppStrings = {
  app: {
    offlineNotice:
      "You are currently offline. Local mesh functions and chess remain available on this island.",
    notFound:
      "Error 404: The requested Captive Portal page was not found on this node.",
    badge: "CAPTIVE PORTAL ACTIVE",
    subtitle: "Offline-First Decentralized Chess Grid",
    portalCopy:
      "Welcome to the offline Captive Portal. You are connected to an isolated mesh island. " +
      "No internet is required. Game state lives on-device; peers sync via Kademlia DHT and LoRa " +
      "when available.",
  },
  chess: {
    checkmateWhiteWins: "Checkmate - White wins",
    checkmateBlackWins: "Checkmate - Black wins",
    stalemate: "Stalemate - draw",
    threefold: "Draw - threefold repetition",
    insufficientMaterial: "Draw - insufficient material",
    draw: "Draw",
    whiteInCheck: "White in check",
    blackInCheck: "Black in check",
    whiteToMove: "White to move",
    blackToMove: "Black to move",
    whiteToMoveReady: "White to move - offline engine ready",
    illegalMove: "Illegal move",
    noteLocalOnly: "Moves stay on-device until a mesh peer is found.",
    noteUndone: "Undid last move (local only)",
    notePlayOffline: "Play offline - pure local chess engine",
    noteSeeking: "Seeking peer via LoRa / mesh island...",
    noteQueuedLora: "Offline: move queued for LoRa / local DHT relay",
    noteAppliedLocal: "Offline: move applied locally (no internet required)",
    noteSubmitting: "Submitting move to mesh...",
    noteConfirmed: "Move confirmed by swarm",
    noteRejected: "Swarm rejected ack - kept local state",
    noteUnreachable: "Mesh unreachable - kept local (DHT island mode)",
    noteNewGame: "New game - local FEN store reset",
    moveHistory: "Move history",
    noMoves: "No moves yet - play offline",
    playOffline: "Play offline",
    seekPeer: "Seek peer via LoRa",
    undo: "Undo",
    newGame: "New game",
    evalDaemonTitle: "Score from the mesh engine sandbox",
    evalLocalTitle: "Local on-device evaluation",
    evalLoading: "loading",
    evalAria: (label) => `Engine evaluation ${label} pawns`,
    evalText: (score) => `Eval ${score}`,
  },
  visualizer: {
    title: "Mesh islands",
    scanning: "Scanning local mesh environment...",
    summary: (islands, peers, name) =>
      `${islands} island${islands === 1 ? "" : "s"} - ${peers} live nodes - ${name}`,
    probing: "Probing Kademlia + LoRa beacons",
    nodes: (n) => `${n} nodes`,
    ping: (latency, hops) => `Ping: ${latency}ms - ${hops} hop${hops === 1 ? "" : "s"}`,
    signal: (rssi) => `Signal strength ${rssi} dBm`,
    signalAria: (rssi, bars) => `Signal strength ${rssi} dBm, ${bars} of 4 bars`,
    hint:
      "Isolated islands exchange games via store-and-forward when a bridge peer appears. " +
      "No WAN required for local play.",
  },
  daemonToast: {
    message:
      "Local daemon unreachable. Island mode active: chess and mesh continue on-device.",
    dismiss: "Dismiss notification",
  },
  ble: {
    unavailable: "BLE unavailable",
    enable: "Enable BLE relay",
    scanning: "Scanning...",
    on: "BLE relay on",
    failed: "BLE failed - retry",
    useAsRelay: "Use this browser as a BLE mesh relay",
    relayingVia: (peer, frames) => `Relaying via ${peer} - ${frames} frames`,
  },
};

const es: AppStrings = {
  app: {
    offlineNotice:
      "Estás sin conexión. Las funciones locales de la malla y el ajedrez siguen disponibles en esta isla.",
    notFound:
      "Error 404: La página solicitada del portal cautivo no se encontró en este nodo.",
    badge: "PORTAL CAUTIVO ACTIVO",
    subtitle: "Red de ajedrez descentralizada sin conexión",
    portalCopy:
      "Bienvenido al portal cautivo sin conexión. Estás conectado a una isla mesh aislada. " +
      "No se necesita internet. El estado de la partida vive en el dispositivo; los pares se " +
      "sincronizan mediante Kademlia DHT y LoRa cuando están disponibles.",
  },
  chess: {
    checkmateWhiteWins: "Jaque mate - Ganan las blancas",
    checkmateBlackWins: "Jaque mate - Ganan las negras",
    stalemate: "Tablas por ahogado",
    threefold: "Tablas - triple repetición",
    insufficientMaterial: "Tablas - material insuficiente",
    draw: "Tablas",
    whiteInCheck: "Blancas en jaque",
    blackInCheck: "Negras en jaque",
    whiteToMove: "Juegan las blancas",
    blackToMove: "Juegan las negras",
    whiteToMoveReady: "Juegan las blancas - motor local listo",
    illegalMove: "Movimiento ilegal",
    noteLocalOnly: "Los movimientos se quedan en el dispositivo hasta encontrar un par de la malla.",
    noteUndone: "Último movimiento deshecho (solo local)",
    notePlayOffline: "Jugar sin conexión - motor local puro",
    noteSeeking: "Buscando par vía LoRa / isla mesh...",
    noteQueuedLora: "Sin conexión: movimiento en cola para retransmisión LoRa / DHT local",
    noteAppliedLocal: "Sin conexión: movimiento aplicado localmente (no se necesita internet)",
    noteSubmitting: "Enviando movimiento a la malla...",
    noteConfirmed: "Movimiento confirmado por el enjambre",
    noteRejected: "El enjambre rechazó la confirmación - estado local conservado",
    noteUnreachable: "Malla inalcanzable - modo local (isla DHT)",
    noteNewGame: "Nueva partida - almacén FEN local reiniciado",
    moveHistory: "Historial de movimientos",
    noMoves: "Sin movimientos aún - juega sin conexión",
    playOffline: "Jugar sin conexión",
    seekPeer: "Buscar par vía LoRa",
    undo: "Deshacer",
    newGame: "Nueva partida",
    evalDaemonTitle: "Puntuación del motor de la malla",
    evalLocalTitle: "Evaluación local en el dispositivo",
    evalLoading: "cargando",
    evalAria: (label) => `Evaluación del motor: ${label} peones`,
    evalText: (score) => `Eval. ${score}`,
  },
  visualizer: {
    title: "Islas mesh",
    scanning: "Explorando el entorno mesh local...",
    summary: (islands, peers, name) =>
      `${islands} isla${islands === 1 ? "" : "s"} - ${peers} nodos activos - ${name}`,
    probing: "Sondeando balizas Kademlia + LoRa",
    nodes: (n) => `${n} nodo${n === 1 ? "" : "s"}`,
    ping: (latency, hops) => `Ping: ${latency}ms - ${hops} salto${hops === 1 ? "" : "s"}`,
    signal: (rssi) => `Intensidad de señal: ${rssi} dBm`,
    signalAria: (rssi, bars) => `Intensidad de señal: ${rssi} dBm, ${bars} de 4 barras`,
    hint:
      "Las islas aisladas intercambian partidas por almacenamiento y reenvío cuando aparece " +
      "un par puente. No se necesita WAN para jugar localmente.",
  },
  daemonToast: {
    message:
      "Demonio local inalcanzable. Modo isla activo: el ajedrez y la malla siguen funcionando en el dispositivo.",
    dismiss: "Descartar notificación",
  },
  ble: {
    unavailable: "BLE no disponible",
    enable: "Activar retransmisión BLE",
    scanning: "Buscando...",
    on: "Retransmisión BLE activa",
    failed: "BLE falló - reintentar",
    useAsRelay: "Usar este navegador como retransmisor BLE de la malla",
    relayingVia: (peer, frames) => `Retransmitiendo vía ${peer} - ${frames} tramas`,
  },
};

const fr: AppStrings = {
  app: {
    offlineNotice:
      "Vous êtes hors ligne. Les fonctions mesh locales et les échecs restent disponibles sur cette île.",
    notFound:
      "Erreur 404 : la page demandée du portail captif est introuvable sur ce nœud.",
    badge: "PORTAIL CAPTIF ACTIF",
    subtitle: "Grille d'échecs décentralisée hors ligne",
    portalCopy:
      "Bienvenue sur le portail captif hors ligne. Vous êtes connecté à une île mesh isolée. " +
      "Aucune connexion internet requise. L'état de la partie reste sur l'appareil ; les pairs se " +
      "synchronisent via Kademlia DHT et LoRa quand ils sont disponibles.",
  },
  chess: {
    checkmateWhiteWins: "Échec et mat - Les blancs gagnent",
    checkmateBlackWins: "Échec et mat - Les noirs gagnent",
    stalemate: "Pat - nulle",
    threefold: "Nulle - triple répétition",
    insufficientMaterial: "Nulle - matériel insuffisant",
    draw: "Nulle",
    whiteInCheck: "Les blancs sont en échec",
    blackInCheck: "Les noirs sont en échec",
    whiteToMove: "Trait aux blancs",
    blackToMove: "Trait aux noirs",
    whiteToMoveReady: "Trait aux blancs - moteur local prêt",
    illegalMove: "Coup illégal",
    noteLocalOnly: "Les coups restent sur l'appareil jusqu'à trouver un pair du mesh.",
    noteUndone: "Dernier coup annulé (local uniquement)",
    notePlayOffline: "Jouer hors ligne - moteur purement local",
    noteSeeking: "Recherche de pair via LoRa / île mesh...",
    noteQueuedLora: "Hors ligne : coup en file pour relais LoRa / DHT local",
    noteAppliedLocal: "Hors ligne : coup appliqué localement (aucune connexion requise)",
    noteSubmitting: "Envoi du coup au mesh...",
    noteConfirmed: "Coup confirmé par l'essaim",
    noteRejected: "Accusé rejeté par l'essaim - état local conservé",
    noteUnreachable: "Mesh injoignable - mode local (île DHT)",
    noteNewGame: "Nouvelle partie - magasin FEN local réinitialisé",
    moveHistory: "Historique des coups",
    noMoves: "Aucun coup - jouez hors ligne",
    playOffline: "Jouer hors ligne",
    seekPeer: "Chercher un pair via LoRa",
    undo: "Annuler",
    newGame: "Nouvelle partie",
    evalDaemonTitle: "Score du moteur du mesh",
    evalLocalTitle: "Évaluation locale sur l'appareil",
    evalLoading: "chargement",
    evalAria: (label) => `Évaluation du moteur : ${label} pions`,
    evalText: (score) => `Éval. ${score}`,
  },
  visualizer: {
    title: "Îles mesh",
    scanning: "Analyse de l'environnement mesh local...",
    summary: (islands, peers, name) =>
      `${islands} île${islands === 1 ? "" : "s"} - ${peers} nœuds actifs - ${name}`,
    probing: "Sondage des balises Kademlia + LoRa",
    nodes: (n) => `${n} nœud${n === 1 ? "" : "s"}`,
    ping: (latency, hops) => `Ping : ${latency}ms - ${hops} saut${hops === 1 ? "" : "s"}`,
    signal: (rssi) => `Force du signal : ${rssi} dBm`,
    signalAria: (rssi, bars) => `Force du signal : ${rssi} dBm, ${bars} barres sur 4`,
    hint:
      "Les îles isolées échangent des parties en différé dès qu'un pair relais apparaît. " +
      "Aucun WAN requis pour jouer en local.",
  },
  daemonToast: {
    message:
      "Démon local injoignable. Mode île actif : les échecs et le mesh continuent sur l'appareil.",
    dismiss: "Ignorer la notification",
  },
  ble: {
    unavailable: "BLE indisponible",
    enable: "Activer le relais BLE",
    scanning: "Recherche...",
    on: "Relais BLE actif",
    failed: "Échec BLE - réessayer",
    useAsRelay: "Utiliser ce navigateur comme relais BLE du mesh",
    relayingVia: (peer, frames) => `Relais via ${peer} - ${frames} trames`,
  },
};

const de: AppStrings = {
  app: {
    offlineNotice:
      "Du bist offline. Lokale Mesh-Funktionen und Schach bleiben auf dieser Insel verfügbar.",
    notFound:
      "Fehler 404: Die angeforderte Captive-Portal-Seite wurde auf diesem Knoten nicht gefunden.",
    badge: "CAPTIVE-PORTAL AKTIV",
    subtitle: "Offline-fähiges dezentrales Schachgitter",
    portalCopy:
      "Willkommen beim Offline-Captive-Portal. Du bist mit einer isolierten Mesh-Insel verbunden. " +
      "Kein Internet erforderlich. Der Spielstand bleibt auf dem Gerät; Peers synchronisieren sich " +
      "bei Verfügbarkeit über Kademlia DHT und LoRa.",
  },
  chess: {
    checkmateWhiteWins: "Schachmatt - Weiß gewinnt",
    checkmateBlackWins: "Schachmatt - Schwarz gewinnt",
    stalemate: "Patt - Remis",
    threefold: "Remis - dreifache Zugwiederholung",
    insufficientMaterial: "Remis - unzureichendes Material",
    draw: "Remis",
    whiteInCheck: "Weiß steht im Schach",
    blackInCheck: "Schwarz steht im Schach",
    whiteToMove: "Weiß am Zug",
    blackToMove: "Schwarz am Zug",
    whiteToMoveReady: "Weiß am Zug - lokale Engine bereit",
    illegalMove: "Ungültiger Zug",
    noteLocalOnly: "Züge bleiben auf dem Gerät, bis ein Mesh-Peer gefunden wird.",
    noteUndone: "Letzten Zug zurückgenommen (nur lokal)",
    notePlayOffline: "Offline spielen - rein lokale Engine",
    noteSeeking: "Suche Peer über LoRa / Mesh-Insel...",
    noteQueuedLora: "Offline: Zug für LoRa-/lokales DHT-Relay vorgemerkt",
    noteAppliedLocal: "Offline: Zug lokal angewendet (kein Internet erforderlich)",
    noteSubmitting: "Sende Zug ans Mesh...",
    noteConfirmed: "Zug vom Schwarm bestätigt",
    noteRejected: "Schwarm hat Bestätigung abgelehnt - lokaler Stand behalten",
    noteUnreachable: "Mesh unerreichbar - lokaler Modus (DHT-Insel)",
    noteNewGame: "Neue Partie - lokaler FEN-Speicher zurückgesetzt",
    moveHistory: "Zughistorie",
    noMoves: "Noch keine Züge - offline spielen",
    playOffline: "Offline spielen",
    seekPeer: "Peer über LoRa suchen",
    undo: "Rückgängig",
    newGame: "Neue Partie",
    evalDaemonTitle: "Wertung der Mesh-Engine",
    evalLocalTitle: "Lokale Bewertung auf dem Gerät",
    evalLoading: "wird geladen",
    evalAria: (label) => `Engine-Bewertung: ${label} Bauern`,
    evalText: (score) => `Wert. ${score}`,
  },
  visualizer: {
    title: "Mesh-Inseln",
    scanning: "Scanne lokale Mesh-Umgebung...",
    summary: (islands, peers, name) =>
      `${islands} Insel${islands === 1 ? "" : "n"} - ${peers} aktive Knoten - ${name}`,
    probing: "Suche Kademlia- + LoRa-Beacons",
    nodes: (n) => `${n} Knoten`,
    ping: (latency, hops) => `Ping: ${latency}ms - ${hops} Hop${hops === 1 ? "" : "s"}`,
    signal: (rssi) => `Signalstärke: ${rssi} dBm`,
    signalAria: (rssi, bars) => `Signalstärke: ${rssi} dBm, ${bars} von 4 Balken`,
    hint:
      "Isolierte Inseln tauschen Partien per Store-and-Forward aus, sobald ein Brücken-Peer " +
      "erscheint. Kein WAN für lokales Spiel nötig.",
  },
  daemonToast: {
    message:
      "Lokaler Daemon unerreichbar. Inselmodus aktiv: Schach und Mesh laufen weiter auf dem Gerät.",
    dismiss: "Benachrichtigung schließen",
  },
  ble: {
    unavailable: "BLE nicht verfügbar",
    enable: "BLE-Relay aktivieren",
    scanning: "Suche...",
    on: "BLE-Relay aktiv",
    failed: "BLE fehlgeschlagen - erneut versuchen",
    useAsRelay: "Diesen Browser als BLE-Mesh-Relay nutzen",
    relayingVia: (peer, frames) => `Relay über ${peer} - ${frames} Frames`,
  },
};

const STRINGS: Record<AppLang, AppStrings> = { en, es, fr, de };

export function getStrings(lang: AppLang): AppStrings {
  return STRINGS[lang] ?? en;
}

export function initialLang(): AppLang {
  try {
    const saved = window.localStorage.getItem(LANG_KEY);
    if (saved === "es" || saved === "fr" || saved === "de" || saved === "en") {
      return saved;
    }
  } catch {
    // Storage unavailable; fall through to browser language.
  }
  const nav = typeof navigator !== "undefined" ? navigator.language.slice(0, 2) : "en";
  if (nav === "es" || nav === "fr" || nav === "de") return nav;
  return "en";
}

export function persistLang(lang: AppLang): void {
  try {
    window.localStorage.setItem(LANG_KEY, lang);
  } catch {
    // Storage unavailable; language still applies for this session.
  }
}
