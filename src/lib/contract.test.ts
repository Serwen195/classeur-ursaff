// Contrat TypeScript ↔ Rust : les commandes appelées par `api.ts` doivent exister côté Rust avec
// exactement les mêmes noms d'arguments (en camelCase). Une faute de frappe ne se verrait sinon
// que dans l'application réelle, les autres tests utilisant un faux backend ou du JSON écrit à la main.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const camel = (s: string) => s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());

/** Commandes `#[tauri::command]` : nom → arguments fournis par l'interface (hors AppHandle/State injectés). */
function rustCommands(): Map<string, string[]> {
  const src = readFileSync('src-tauri/src/commands.rs', 'utf8');
  const out = new Map<string, string[]>();
  const re = /#\[tauri::command\]\s*pub\s+(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(([\s\S]*?)\)\s*(?:->[^{]*)?\{/g;
  for (const m of src.matchAll(re)) {
    const params = m[2]
      .split(/,(?![^<]*>)/) // ne coupe pas à l'intérieur de <...>
      .map((p) => p.trim())
      .filter(Boolean)
      .filter((p) => !/:\s*(?:&?\s*)?AppHandle\b/.test(p) && !/:\s*State\b/.test(p))
      .map((p) => camel(p.split(':')[0].trim()));
    out.set(m[1], params);
  }
  return out;
}

/** Appels `call('commande', { a, b: x })` de api.ts → nom → clés. */
function tsCalls(): Map<string, string[]> {
  const src = readFileSync('src/lib/api.ts', 'utf8');
  const out = new Map<string, string[]>();
  for (const m of src.matchAll(/call(?:<[^>]*>)?\(\s*'([a-z_]+)'\s*(?:,\s*\{([^}]*)\})?\s*\)/g)) {
    const keys = (m[2] ?? '')
      .split(',')
      .map((k) => k.trim().split(':')[0].trim())
      .filter(Boolean);
    out.set(m[1], keys);
  }
  return out;
}

describe('contrat api.ts ↔ commands.rs', () => {
  const rust = rustCommands();
  const ts = tsCalls();

  it('lit bien les deux côtés (garde-fou du test lui-même)', () => {
    expect(rust.size).toBeGreaterThanOrEqual(19);
    expect(ts.size).toBeGreaterThanOrEqual(19);
    expect(rust.get('save_month')).toEqual(['input']);
    expect(rust.get('change_passphrase')).toEqual(['oldPassphrase', 'newPassphrase']);
  });

  it("chaque commande appelée par l'interface existe côté Rust", () => {
    for (const name of ts.keys()) expect(rust.has(name), `commande inconnue côté Rust : ${name}`).toBe(true);
  });

  it("chaque commande Rust est utilisée par l'interface (pas de commande exposée pour rien)", () => {
    for (const name of rust.keys()) expect(ts.has(name), `commande Rust jamais appelée : ${name}`).toBe(true);
  });

  it("les noms d'arguments sont identiques", () => {
    for (const [name, keys] of ts) {
      expect([...keys].sort(), `arguments de ${name}`).toEqual([...(rust.get(name) ?? [])].sort());
    }
  });

  it("la liste des commandes enregistrées dans lib.rs correspond à commands.rs", () => {
    const lib = readFileSync('src-tauri/src/lib.rs', 'utf8');
    const registered = [...lib.matchAll(/\$crate::commands::(\w+),/g)].map((m) => m[1]);
    expect(new Set(registered)).toEqual(new Set(rust.keys()));
  });

  it("les capacités n'accordent aucun accès fichier à la page", () => {
    const cap = JSON.parse(readFileSync('src-tauri/capabilities/default.json', 'utf8')) as { permissions: string[] };
    for (const p of cap.permissions) expect(p, 'permission trop large').not.toMatch(/^(fs|shell|opener|http|dialog):/);
  });
});

// ───────── formes JSON : types.ts ↔ contract.json (le même fichier est vérifié côté Rust) ─────────

interface JsonContract {
  objects: Record<string, string[]>;
  enums: Record<string, string[]>;
}
const contract = JSON.parse(readFileSync('src/lib/contract.json', 'utf8')) as JsonContract;
const types = readFileSync('src/lib/types.ts', 'utf8');

describe('contrat JSON types.ts ↔ contract.json', () => {
  for (const [name, fields] of Object.entries(contract.objects)) {
    it(`interface ${name}`, () => {
      const m = new RegExp(`export interface ${name} \\{([\\s\\S]*?)\\n\\}`).exec(types);
      expect(m, `interface ${name} introuvable dans types.ts`).not.toBeNull();
      const declared = [...m![1].matchAll(/^\s*(\w+)\??:/gm)].map((x) => x[1]).sort();
      expect(declared).toEqual([...fields].sort());
    });
  }
  for (const [name, values] of Object.entries(contract.enums)) {
    it(`type ${name}`, () => {
      const m = new RegExp(`export type ${name} =([^;]+);`).exec(types);
      expect(m, `type ${name} introuvable dans types.ts`).not.toBeNull();
      const declared = [...m![1].matchAll(/'([a-z_]+)'/g)].map((x) => x[1]).sort();
      expect(declared).toEqual([...values].sort());
    });
  }
});
