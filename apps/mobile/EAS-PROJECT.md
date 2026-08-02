# EAS / Expo project identity

- App `slug` in `app.json` is **mitsuro**.
- Apple bundle id remains **io.krusty.mobile** (TestFlight / App Store identity freeze).
- `extra.eas.projectId` must point at an Expo project whose **slug is also mitsuro**.

If EAS fails with:

```
Slug for project identified by "extra.eas.projectId" (krusty) does not match the "slug" field (mitsuro)
```

the Expo project is still named `krusty`. Rename it in the Expo dashboard (or with EAS CLI) to `mitsuro`, **or** create a new Expo project with slug `mitsuro` and update `projectId`. Do not change the iOS bundle id as part of that rename.
