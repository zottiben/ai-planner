# Icons

`mark.svg` is the source. Everything else in here is generated from it and should not
be hand-edited - change the SVG and regenerate.

The mark is three columns of a board, descending, with the rightmost already cleared.
It is deliberately geometric: the only hard constraint on an app icon is that it still
reads at 16px in a dock or a taskbar, and an illustration at that size is a smudge.

## Regenerating

`tauri icon` wants a square PNG, so the SVG is rasterised first. Rather than require
ImageMagick or Inkscape, this uses the Chromium that Playwright already installs for
the UI checks:

```sh
# 1. Rasterise the mark at 1024x1024
node - <<'EOF'
import { chromium } from "playwright";
import { readFileSync } from "node:fs";
const svg = readFileSync("icons/mark.svg", "utf8");
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1024, height: 1024 } });
await page.setContent(`<body style="margin:0">${svg}</body>`);
await page.locator("svg").screenshot({ path: "icons/mark.png", omitBackground: true });
await browser.close();
EOF

# 2. Generate the platform set
npx @tauri-apps/cli@2 icon icons/mark.png -o icons

# 3. The mobile sets are not shipped
rm -rf icons/android icons/ios
```

Check the result at 32px before committing. That, not the 1024px version, is the size
it will actually be seen at.
