- **An area with a Landscaper sky publishes with its sky.** Publish, a module's publish and Try on dev send `ls_skies.hak`'s
  files as `skies_files` (a check names them in `skies_manifest`), the package for a module has them in `haks/ls_skies/`, and
  `dev-pack` copies the hak and lists it: the area's sky row is in that hak's `skyboxes.2da`. The service routes give it as
  `GET /api/service/skies`. Areas without one send what they did before.
