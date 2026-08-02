# EAS / Expo project identity

- App display `name` is **Mitsuro**.
- App `slug` is currently **`krusty`** so it matches the existing Expo project
  bound by `extra.eas.projectId` (`6e327449-af3c-4138-b1c4-7ceca2baf243`).
- Apple bundle id remains **io.krusty.mobile** (TestFlight / App Store freeze).

## Why slug is not `mitsuro` yet

EAS fails when `app.json` slug disagrees with the Expo project slug for the
linked `projectId`:

```
Slug for project identified by "extra.eas.projectId" (krusty)
does not match the "slug" field (mitsuro)
```

To switch the slug to `mitsuro` permanently:

1. Rename the Expo project slug `krusty` → `mitsuro` in the Expo dashboard
   (or create a new Expo project with slug `mitsuro`).
2. If you create a new project, update `extra.eas.projectId`.
3. Set `app.json` `slug` to `mitsuro`.
4. Do **not** change the iOS bundle id as part of that rename.

Until that rename is done, keep `slug: "krusty"` so TestFlight CI can build.
