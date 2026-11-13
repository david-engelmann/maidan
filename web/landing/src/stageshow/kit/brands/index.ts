/** Local product artwork. No network lookup or runtime icon service. */
// Trimmed from Stageshow's registry to the three marks the landing shows, and
// imported ?inline: the server serves only the landing's named files
// (app.js, style.css and the rest), so a mark emitted as its own file would 404.
import slack from './slack.svg?inline'
import github from './github.svg?inline'
import githubDark from './github-dark.svg?inline'
import linear from './linear.svg?inline'
import linearDark from './linear-dark.svg?inline'

export type BrandAsset = {
  name: string
  category: 'Payments' | 'Banks' | 'Apps' | 'Crypto'
  light: string
  dark: string
  tileArtwork?: string
  tileBackground?: string
  proportion: number
  scale: number
}

export const brandMarks = {
  'slack': {
    name: "Slack", category: 'Apps',
    light: slack,
    dark: slack,
    proportion: 1, scale: 0.64,
  },
  'github': {
    name: "GitHub", category: 'Apps',
    light: github,
    dark: githubDark,
    proportion: 1, scale: 0.64,
  },
  'linear': {
    name: "Linear", category: 'Apps',
    light: linear,
    dark: linearDark,
    proportion: 1, scale: 0.64,
  },
} as const satisfies Record<string, BrandAsset>

export type BrandName = keyof typeof brandMarks
