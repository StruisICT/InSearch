# Changelog

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
