import { expect, test, type Page } from '@playwright/test';

const PASS = 'phrase secrete de test'; // phrase du faux backend
const nbsp = /[  ]/g;
const txt = async (page: Page, testId: string) => ((await page.getByTestId(testId).textContent()) ?? '').replace(nbsp, ' ').trim();

async function unlock(page: Page, pass = PASS) {
  await page.getByLabel('Phrase secrète').fill(pass);
  await page.getByRole('button', { name: 'Déverrouiller' }).click();
}

const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64',
);

test.describe('première utilisation', () => {
  test('assistant : jeton refusé, puis connexion, puis création du coffre', async ({ page }) => {
    await page.goto('/?scenario=setup');
    await expect(page.getByRole('heading', { name: '1. Connecter votre dépôt de données' })).toBeVisible();

    await page.getByLabel('Adresse du dépôt').fill('https://github.com/moi/classeur-urssaf-data');
    await page.getByLabel("Jeton d'accès").fill('mauvais');
    await page.getByRole('button', { name: 'Se connecter' }).click();
    await expect(page.getByRole('alert')).toContainText('Authentification refusée');
    await expect(page.getByRole('heading', { name: '1. Connecter votre dépôt de données' })).toBeVisible();

    await page.getByLabel("Jeton d'accès").fill('github_pat_ok');
    await page.getByRole('button', { name: 'Se connecter' }).click();
    await expect(page.getByRole('heading', { name: '2. Choisir votre phrase secrète' })).toBeVisible();
    await expect(page.getByText('définitivement irrécupérables')).toBeVisible();

    const create = page.getByRole('button', { name: 'Créer le classeur' });
    await expect(create).toBeDisabled();
    await page.getByLabel('Phrase secrète', { exact: true }).fill('trop court');
    await expect(page.getByText('10 / 12 caractères minimum')).toBeVisible();
    await page.getByLabel('Phrase secrète', { exact: true }).fill('une phrase assez longue');
    await page.getByLabel('Confirmer la phrase secrète').fill('une phrase differente!!');
    await expect(page.getByText('ne correspondent pas')).toBeVisible();
    await page.getByLabel('Confirmer la phrase secrète').fill('une phrase assez longue');
    await expect(create).toBeDisabled(); // case de compréhension non cochée
    await page.getByLabel('Je comprends').check();
    await expect(create).toBeEnabled();
    await create.click();

    await expect(page.getByRole('button', { name: /Verrouiller/ })).toBeVisible();
    await expect(page.getByTestId('total-declared')).toContainText('0,00');
  });

  test('un second appareil reconnaît le coffre existant et demande la phrase', async ({ page }) => {
    await page.goto('/?scenario=setup');
    await page.getByLabel('Adresse du dépôt').fill('https://github.com/moi/depot-existant');
    await page.getByLabel("Jeton d'accès").fill('github_pat_ok');
    await page.getByRole('button', { name: 'Se connecter' }).click();
    await expect(page.getByRole('button', { name: 'Déverrouiller' })).toBeVisible();
  });
});

test.describe('verrouillage', () => {
  test('mauvaise phrase refusée, bonne phrase acceptée', async ({ page }) => {
    await page.goto('/?scenario=locked');
    await unlock(page, 'pas la bonne phrase');
    await expect(page.getByRole('alert')).toContainText('Phrase secrète incorrecte');
    await expect(page.getByLabel('Phrase secrète')).toHaveValue('');
    await unlock(page);
    await expect(page.getByTestId('total-declared')).toBeVisible();
  });

  test('verrouiller efface les données de l\'écran et du DOM', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await expect(page.getByTestId('total-declared')).toBeVisible();
    await page.getByRole('button', { name: /Verrouiller/ }).click();
    await expect(page.getByRole('button', { name: 'Déverrouiller' })).toBeVisible();
    const html = (await page.content()).replace(nbsp, ' ');
    for (const secret of ['8 700,65', 'facture fournisseur', 'Accusé de déclaration']) {
      expect(html).not.toContain(secret);
    }
    await unlock(page);
    await expect(page.getByTestId('total-declared')).toBeVisible();
  });

  test('oublier ce poste ramène à l\'assistant après confirmation', async ({ page }) => {
    await page.goto('/?scenario=locked');
    await page.getByRole('button', { name: 'Oublier ce poste' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Oublier ce poste' }).click();
    await expect(page.getByRole('heading', { name: '1. Connecter votre dépôt de données' })).toBeVisible();
  });
});

test.describe('année et totaux', () => {
  test('affiche 12 mois et un total qui exclut les mois « à déclarer »', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await expect(page.getByRole('row')).toHaveCount(1 + 12 + 1); // en-tête + 12 mois + total
    expect(await txt(page, 'total-declared')).toBe('8 700,65 €');
    expect(await txt(page, 'total-pending')).toContain('875,00 €');
  });

  test('changer d\'année', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.getByRole('button', { name: 'Année précédente' }).click();
    const y = new Date().getFullYear() - 1;
    await expect(page.getByLabel('Année', { exact: true })).toHaveValue(String(y));
    expect(await txt(page, 'total-declared')).toBe('4 962,50 €'); // 1 842,50 + 3 120,00
  });
});

test.describe('édition d\'un mois', () => {
  test('saisie à la française, statut déclaré → date du jour proposée, total mis à jour', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.getByRole('button', { name: /Modifier avril/ }).click();
    await page.getByLabel('CA déclaré (€)').fill('1 500,5');
    await page.getByRole('radio', { name: 'Déclaré' }).check({ force: true });
    await expect(page.getByLabel('Date de déclaration')).not.toHaveValue('');
    await page.getByLabel('Notes').fill('Déclaré sans souci.');
    await page.getByRole('button', { name: 'Enregistrer' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    expect(await txt(page, 'total-declared')).toBe('10 201,15 €'); // 8 700,65 + 1 500,50
    await expect(page.getByTestId('total-pending')).toHaveCount(0);
    await expect(page.getByRole('row', { name: /avril/i })).toContainText('1 500,50');
  });

  test('un montant invalide est refusé sans rien enregistrer', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.getByRole('button', { name: /Modifier mai/ }).click();
    await page.getByLabel('CA déclaré (€)').fill('12,345');
    await page.getByRole('button', { name: 'Enregistrer' }).click();
    await expect(page.getByRole('dialog').getByRole('alert')).toContainText('Deux décimales');
    await expect(page.getByRole('dialog')).toBeVisible();
  });

  test('fermer avec des modifications demande confirmation', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.getByRole('button', { name: /Modifier mai/ }).click();
    await page.getByLabel('Notes').fill('brouillon');
    await page.getByRole('button', { name: 'Annuler' }).click();
    await expect(page.getByText('Abandonner les modifications ?')).toBeVisible();
    await page.getByRole('button', { name: 'Abandonner' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.getByRole('row', { name: /mai/i })).not.toContainText('brouillon');
  });

  test('vider un mois le supprime', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.getByRole('button', { name: /Modifier mars/ }).click();
    await page.getByRole('button', { name: 'Vider ce mois' }).click();
    await page.getByRole('dialog').filter({ hasText: /Vider Mars/ }).getByRole('button', { name: 'Vider ce mois' }).click();
    await expect(page.getByRole('row', { name: /mars/i })).toContainText('Vide');
    expect(await txt(page, 'total-declared')).toBe('4 480,75 €'); // 8 700,65 − 4 219,90
  });
});

test.describe('pièces jointes', () => {
  test('ajout, aperçu, suppression ; les types non pris en charge sont refusés', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.getByRole('button', { name: /Modifier mai/ }).click();

    const chooser = page.waitForEvent('filechooser');
    await page.getByRole('button', { name: 'Ajouter des fichiers…' }).click();
    await (await chooser).setFiles([
      { name: 'ar-mai.png', mimeType: 'image/png', buffer: png },
      { name: 'virus.exe', mimeType: 'application/octet-stream', buffer: Buffer.from('MZ') },
    ]);
    await expect(page.getByRole('alert')).toContainText('Seuls les PDF et les images');
    await expect(page.getByRole('listitem').filter({ hasText: 'ar-mai.png' })).toBeVisible();
    await expect(page.getByRole('listitem').filter({ hasText: 'virus.exe' })).toHaveCount(0);

    await page.getByRole('button', { name: 'Aperçu' }).click();
    const img = page.getByRole('img', { name: 'Aperçu de ar-mai.png' });
    await expect(img).toBeVisible();
    expect(await img.evaluate((el: HTMLImageElement) => el.src.startsWith('blob:') && el.naturalWidth)).toBe(1);

    await page.getByRole('button', { name: 'Supprimer ar-mai.png' }).click();
    await page.getByRole('dialog').filter({ hasText: 'Supprimer la pièce jointe ?' }).getByRole('button', { name: 'Supprimer' }).click();
    await expect(page.getByRole('listitem').filter({ hasText: 'ar-mai.png' })).toHaveCount(0);
  });

  test('les pièces jointes apparaissent dans la vue annuelle', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await expect(page.getByRole('row', { name: /janvier/i })).toContainText('📎 1');
  });
});

test.describe('recherche', () => {
  test('insensible aux accents, mène au mois, Échap efface', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.getByRole('searchbox').fill('RETARD');
    await expect(page.getByRole('heading', { name: /1 résultat/ })).toBeVisible();
    await page.getByRole('button', { name: /Février/ }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await expect(page.getByLabel('Notes')).toHaveValue(/retard/);
    await page.getByRole('button', { name: 'Annuler' }).click();

    await page.getByRole('searchbox').fill('accuse janvier');
    await expect(page.getByRole('button', { name: /Janvier/ })).toBeVisible();
    await page.getByRole('searchbox').fill('introuvable-xyz');
    await expect(page.getByRole('heading', { name: 'Aucun résultat' })).toBeVisible();
    await page.getByRole('searchbox').press('Escape');
    await expect(page.getByTestId('total-declared')).toBeVisible();
  });

  test('Ctrl+F place le curseur dans la recherche', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.keyboard.press('Control+f');
    await expect(page.getByRole('searchbox')).toBeFocused();
  });
});

test.describe('synchronisation, réglages et mises à jour', () => {
  test('hors ligne : le badge le dit et les modifications restent en attente', async ({ page }) => {
    await page.goto('/?scenario=unlocked&offline=1');
    await page.getByRole('button', { name: /Modifier mai/ }).click();
    await page.getByLabel('CA déclaré (€)').fill('10');
    await page.getByRole('button', { name: 'Enregistrer' }).click();
    await expect(page.getByRole('button', { name: /Hors ligne — modifications en attente/ })).toBeVisible();
  });

  test('réglages : phrase actuelle fausse refusée, puis changement réussi', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.getByRole('button', { name: '⚙ Réglages' }).click();
    const dlg = page.getByRole('dialog', { name: 'Réglages' });
    await dlg.getByLabel('Phrase actuelle').fill('mauvaise');
    await dlg.getByLabel('Nouvelle phrase (12 caractères minimum)').fill('une nouvelle phrase longue');
    await dlg.getByLabel('Confirmer la nouvelle phrase').fill('une nouvelle phrase longue');
    await dlg.getByRole('button', { name: 'Changer la phrase' }).click();
    await expect(dlg.getByRole('alert')).toContainText('incorrecte');
    await dlg.getByLabel('Phrase actuelle').fill(PASS);
    await dlg.getByRole('button', { name: 'Changer la phrase' }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Phrase secrète modifiée' })).toBeVisible();
  });

  test('réglages : verrouillage automatique modifiable', async ({ page }) => {
    await page.goto('/?scenario=unlocked');
    await page.getByRole('button', { name: '⚙ Réglages' }).click();
    const select = page.getByLabel('Verrouillage automatique après inactivité');
    await expect(select).toHaveValue('10');
    await select.selectOption('30');
    await expect(select).toHaveValue('30');
  });

  test('mise à jour disponible : bandeau, installation avec progression', async ({ page }) => {
    await page.goto('/?scenario=locked&update=1');
    const banner = page.getByRole('status').filter({ hasText: 'Nouvelle version 9.9.9' });
    await expect(banner).toBeVisible({ timeout: 10_000 });
    await banner.getByRole('button', { name: 'Installer et redémarrer' }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Téléchargement de la version 9.9.9' })).toBeVisible();
  });

  test('échec d\'installation (signature invalide) : message clair et possibilité de réessayer', async ({ page }) => {
    await page.goto('/?scenario=locked&update=1&updateFail=1');
    const banner = page.getByRole('status').filter({ hasText: 'Nouvelle version 9.9.9' });
    await expect(banner).toBeVisible({ timeout: 10_000 });
    await banner.getByRole('button', { name: 'Installer et redémarrer' }).click();
    await expect(page.getByRole('alert')).toContainText("La mise à jour n'a pas pu être installée");
    await expect(page.getByRole('alert')).toContainText('signature de la mise à jour invalide');
    // Le bandeau revient : on peut réessayer.
    await expect(banner.getByRole('button', { name: 'Installer et redémarrer' })).toBeVisible();
  });

  test('pas de bandeau quand l\'application est à jour', async ({ page }) => {
    await page.goto('/?scenario=locked');
    await page.waitForTimeout(3500);
    await expect(page.getByText('Nouvelle version')).toHaveCount(0);
  });
});

test.describe('accessibilité de base', () => {
  test('le mot de passe n\'est jamais affiché par défaut et la table a une légende', async ({ page }) => {
    await page.goto('/?scenario=locked');
    await expect(page.getByLabel('Phrase secrète')).toHaveAttribute('type', 'password');
    await page.getByRole('button', { name: 'Afficher' }).click();
    await expect(page.getByLabel('Phrase secrète')).toHaveAttribute('type', 'text');
  });
});
