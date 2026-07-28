import { resolve } from 'node:path'
import { expect, test as setup } from '@playwright/test'

/**
 * Browser login, run once before the browser suite.
 *
 * Drives the real form rather than injecting a cookie, so a regression in the
 * login page fails here instead of silently passing every downstream spec. The
 * resulting `storageState` is reused by the chromium project.
 */
const STORAGE_STATE = resolve(__dirname, '../../test-results/ui/.auth/admin.json')

setup('authenticate as admin', async ({ page }) => {
  const username = process.env.ADMIN_USERNAME || 'admin'
  const password = process.env.ADMIN_PASSWORD || 'ci-admin-password-1234'

  await page.goto('/login')

  await page.getByLabel('Username').fill(username)
  await page.getByLabel('Password').fill(password)
  await page.getByRole('button', { name: 'Sign in' }).click()

  // The form navigates away from /login on success.
  await expect(page).not.toHaveURL(/\/login/)

  await page.context().storageState({ path: STORAGE_STATE })
})

export { STORAGE_STATE }
