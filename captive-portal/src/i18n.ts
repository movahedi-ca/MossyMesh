/**
 * Captive portal i18n: English plus Spanish, French, and German.
 * Kept dependency-free on purpose; the portal must render before any
 * bundle beyond the landing page is available.
 */

export type PortalLang = "en" | "es" | "fr" | "de";

export const PORTAL_LANGS: PortalLang[] = ["en", "es", "fr", "de"];

export const LANG_LABEL: Record<PortalLang, string> = {
  en: "English",
  es: "Español",
  fr: "Français",
  de: "Deutsch",
};

export interface PortalStrings {
  badge: string;
  subtitle: string;
  lede: string;
  enterGrid: string;
  viewMesh: string;
  features: { title: string; body: string }[];
  footerNote: string;
  languageLabel: string;
  themeToLightAria: string;
  themeToDarkAria: string;
  themeLightTitle: string;
  themeDarkTitle: string;
}

const en: PortalStrings = {
  badge: "CAPTIVE PORTAL ACTIVE",
  subtitle: "Offline-First Decentralized Chess Grid",
  lede: "Welcome to the offline Captive Portal. You are connected to an isolated mesh island. No internet is required — packets route locally via Kademlia DHT, BLE, and LoRa.",
  enterGrid: "Enter Chess Grid",
  viewMesh: "View Mesh Status",
  features: [
    { title: "Serverless", body: "No ISP, DNS, or cloud dependency" },
    { title: "Deterministic", body: "Cross-device state transitions for chess PoC" },
    { title: "Edge-ready", body: "Pi, phone, and ESP32 mesh nodes" },
  ],
  footerNote: "150M asset transfers enabled",
  languageLabel: "Language",
  themeToLightAria: "Switch to light theme",
  themeToDarkAria: "Switch to dark theme",
  themeLightTitle: "Light theme",
  themeDarkTitle: "Dark theme",
};

const es: PortalStrings = {
  badge: "PORTAL CAUTIVO ACTIVO",
  subtitle: "Red de ajedrez descentralizada sin conexión",
  lede: "Bienvenido al Portal Cautivo sin conexión. Estás conectado a una isla mesh aislada. No se necesita internet: los paquetes se enrutan localmente mediante Kademlia DHT, BLE y LoRa.",
  enterGrid: "Entrar a la red de ajedrez",
  viewMesh: "Ver estado de la red",
  features: [
    { title: "Sin servidores", body: "Sin dependencia de ISP, DNS ni nube" },
    { title: "Determinista", body: "Transiciones de estado entre dispositivos para la PoC de ajedrez" },
    { title: "Lista para el borde", body: "Nodos mesh en Pi, teléfono y ESP32" },
  ],
  footerNote: "150M de transferencias de activos habilitadas",
  languageLabel: "Idioma",
  themeToLightAria: "Cambiar al tema claro",
  themeToDarkAria: "Cambiar al tema oscuro",
  themeLightTitle: "Tema claro",
  themeDarkTitle: "Tema oscuro",
};

const fr: PortalStrings = {
  badge: "PORTAIL CAPTIF ACTIF",
  subtitle: "Grille d'échecs décentralisée hors ligne",
  lede: "Bienvenue sur le portail captif hors ligne. Vous êtes connecté à une île mesh isolée. Aucune connexion internet requise : les paquets sont routés localement via Kademlia DHT, BLE et LoRa.",
  enterGrid: "Entrer dans la grille d'échecs",
  viewMesh: "Voir l'état du réseau",
  features: [
    { title: "Sans serveur", body: "Aucune dépendance FAI, DNS ou cloud" },
    { title: "Déterministe", body: "Transitions d'état inter-appareils pour la PoC d'échecs" },
    { title: "Prêt pour la périphérie", body: "Nœuds mesh sur Pi, téléphone et ESP32" },
  ],
  footerNote: "150M de transferts d'actifs activés",
  languageLabel: "Langue",
  themeToLightAria: "Passer au thème clair",
  themeToDarkAria: "Passer au thème sombre",
  themeLightTitle: "Thème clair",
  themeDarkTitle: "Thème sombre",
};

const de: PortalStrings = {
  badge: "CAPTIVE-PORTAL AKTIV",
  subtitle: "Offline-fähiges dezentrales Schachgitter",
  lede: "Willkommen beim Offline-Captive-Portal. Du bist mit einer isolierten Mesh-Insel verbunden. Kein Internet erforderlich — Pakete werden lokal über Kademlia DHT, BLE und LoRa geroutet.",
  enterGrid: "Schachgitter betreten",
  viewMesh: "Mesh-Status ansehen",
  features: [
    { title: "Serverlos", body: "Keine ISP-, DNS- oder Cloud-Abhängigkeit" },
    { title: "Deterministisch", body: "Geräteübergreifende Zustandsübergänge für die Schach-PoC" },
    { title: "Edge-bereit", body: "Mesh-Knoten auf Pi, Smartphone und ESP32" },
  ],
  footerNote: "150M Asset-Transfers aktiviert",
  languageLabel: "Sprache",
  themeToLightAria: "Zum hellen Design wechseln",
  themeToDarkAria: "Zum dunklen Design wechseln",
  themeLightTitle: "Helles Design",
  themeDarkTitle: "Dunkles Design",
};

const STRINGS: Record<PortalLang, PortalStrings> = { en, es, fr, de };

export function getStrings(lang: PortalLang): PortalStrings {
  return STRINGS[lang] ?? en;
}

const LANG_KEY = "mossymesh-portal-lang";

export function initialLang(): PortalLang {
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

export function persistLang(lang: PortalLang): void {
  try {
    window.localStorage.setItem(LANG_KEY, lang);
  } catch {
    // Storage unavailable; language still applies for this session.
  }
}
