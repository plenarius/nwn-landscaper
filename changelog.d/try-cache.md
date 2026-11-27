<!-- markdownlint-disable-file MD041 -->
- **The browser page keeps its stock models.** The page build adds the stock bundle's SHA-256 to `try/site.json`
  (`stockSha256`), so the page keeps the models it fetched in the browser's Cache Storage until the bundle changes. The
  handbook's [Try it in your browser](../docs/handbook/try.md) says where areas are kept, and that areas saved at the old
  address stay there.
