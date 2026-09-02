# Changelog

## [0.6.0](https://github.com/StruisICT/InSearch/compare/v0.5.0...v0.6.0) (2026-09-02)


### Features

* **gui:** opt-in update check (InLook-style) with About controls ([fea621c](https://github.com/StruisICT/InSearch/commit/fea621c3641d380b45544fbb36e56df04c3c776b))

## [0.5.0](https://github.com/StruisICT/InSearch/compare/v0.4.1...v0.5.0) (2026-08-27)


### Features

* **cli:** filters, match modes, block, entry-time, and json/count/-l output ([c4a8c40](https://github.com/StruisICT/InSearch/commit/c4a8c40c7ea0f4ef9ad8cbe6da0dae2be28c6049))
* **gui:** preview pane for the selected result ([e98b381](https://github.com/StruisICT/InSearch/commit/e98b381cad19d88d5250506e0c5ad17157967999))
* **gui:** sortable result columns ([88082c6](https://github.com/StruisICT/InSearch/commit/88082c61264689cce7085e06ca0e5988ea6fdf0a))


### Performance Improvements

* share match paths via Arc&lt;Path&gt;; cache GUI display strings ([c15df06](https://github.com/StruisICT/InSearch/commit/c15df06f2f368a973dae9846ec8c55707ae6a6e1))

## [0.4.1](https://github.com/StruisICT/InSearch/compare/v0.4.0...v0.4.1) (2026-08-26)


### Bug Fixes

* **gui:** working About links, top-bar reorder, drop title subtitle ([8295f58](https://github.com/StruisICT/InSearch/commit/8295f587163057c2bce62238ad5b1e33b8487d32))

## [0.4.0](https://github.com/StruisICT/InSearch/compare/v0.3.1...v0.4.0) (2026-08-23)


### Features

* watch-mode filters, persisted preferences, keyboard shortcuts, faster scans ([df6524c](https://github.com/StruisICT/InSearch/commit/df6524c9367c00b6ba8a68edb2e902ad6277844b))

## [0.3.1](https://github.com/StruisICT/InSearch/compare/v0.3.0...v0.3.1) (2026-08-23)


### Features

* **gui:** About window (Struis ICT, source, license, coffee link) ([16a1e80](https://github.com/StruisICT/InSearch/commit/16a1e80e13b43f8e0f7f42b14fc6989955410415))

## [0.3.0](https://github.com/StruisICT/InSearch/compare/v0.2.2...v0.3.0) (2026-08-22)


### Features

* **gui:** detailed results view, clickable filenames, date filter, copy-all ([175c2d8](https://github.com/StruisICT/InSearch/commit/175c2d8bf55a838d2612efd7d3b53eb176d03a85))
* **gui:** entry-time filter, sleek value-activated filters, calendar, filter-aware saved searches ([1369779](https://github.com/StruisICT/InSearch/commit/1369779cb06fa90c97cbfa74f14aa17a1ac591aa))

## [0.2.2](https://github.com/StruisICT/InSearch/compare/v0.2.1...v0.2.2) (2026-08-22)


### Bug Fixes

* **gui:** exit cleanly when the window/OpenGL context can't be created ([3f4d6e6](https://github.com/StruisICT/InSearch/commit/3f4d6e6b5d3ae52aea79a5cfbde7d6acbf813291))

## [0.2.1](https://github.com/StruisICT/InSearch/compare/v0.2.0...v0.2.1) (2026-08-21)


### Bug Fixes

* **cli:** exit 0 when showing usage (no args) ([f32e53d](https://github.com/StruisICT/InSearch/commit/f32e53d836c394ce42b14a660e611fb04a84fb26))

## [0.2.0](https://github.com/StruisICT/InSearch/compare/v0.1.1...v0.2.0) (2026-08-20)


### Features

* **gui:** bundled app icon ([98101b9](https://github.com/StruisICT/InSearch/commit/98101b901ba02192b075d6882a25f6ce1d841f9d))
* **gui:** focus the search box when launched with a folder ([b119434](https://github.com/StruisICT/InSearch/commit/b119434366e9f54bec2143a247aa0313416e4515))
* **ui:** sleeker ClutterCutter-inspired theme ([fe3921d](https://github.com/StruisICT/InSearch/commit/fe3921de6325de92a81e34a397876be096b83e2d))

## [0.1.1](https://github.com/StruisICT/InSearch/compare/v0.1.0...v0.1.1) (2026-08-20)


### Bug Fixes

* correct stale CI note in AGENTS.md (releases are automated) ([9ac0fe3](https://github.com/StruisICT/InSearch/commit/9ac0fe358bee408023a96b11cade12b010d33826))

## 0.1.0 (2026-08-20)


### Features

* boolean queries, whole-word, case modes, match highlighting ([0fd7a74](https://github.com/StruisICT/InSearch/commit/0fd7a743fc1c09f0b7d7fa73d46772de7adc73bd))
* file filters (name, extension, size, modified date) ([09bb809](https://github.com/StruisICT/InSearch/commit/09bb80924399268b41b4b2518ff41e5db8bf627f))
* initial Inspector Fetchy — real-time content-aware file search ([e079f96](https://github.com/StruisICT/InSearch/commit/e079f967095bc27c957f294a1489e5c13852bda4))
* result actions — open/reveal, copy, export, in-place filter ([c1c5260](https://github.com/StruisICT/InSearch/commit/c1c52604bd30121fb12f3918dfaa7e72fd648004))
* session persistence and live progress ([01b0896](https://github.com/StruisICT/InSearch/commit/01b0896e78a81035869ea449e97d5a8993bb0ff8))


### Bug Fixes

* **ci:** allow platform-conditional dead Status variants on non-Windows ([b9299d5](https://github.com/StruisICT/InSearch/commit/b9299d5a3364cdf934756b87df91460e8a873596))
* **cli:** don't panic on a closed stdout pipe ([07d2b1c](https://github.com/StruisICT/InSearch/commit/07d2b1cd6e2db1b60ddfd9315af1d396d57a9a84))
* defer watch start to avoid scan race; robust context-menu status ([1d853d9](https://github.com/StruisICT/InSearch/commit/1d853d95179dababcf9662e6202dec59b1dc51a5))
* **security:** isolate parser panics, cap document size, bump zip ([f2c7380](https://github.com/StruisICT/InSearch/commit/f2c7380f815b777bae2bf549ed1a8f127ee2ede7))


### Miscellaneous Chores

* release 0.1.0 ([a1528db](https://github.com/StruisICT/InSearch/commit/a1528db0cd2e673c4f2e59eae8fe38b184c4d39b))
