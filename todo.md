add VSCode style tree indentation with recursive setup

Goal is to load a workspace -w with my project structure, and use screen to move around fast.

this is an optional feature with the -w flag usage.


│ ~/proj/                                                                             │
│   pers/                                                                             │
│     scrn/              (.git)                                                       │
│     clippie/           (.git)                                                       │
│     tools/                                                                          │
│       rust-build-tools/ (.git)                                                      │
│   work/                                                                             │
│     project-a/         (.git)


- all should be one tree. it should NOT move up the opened screens into some top group like is now. 
- when searching "/", trim away those not found (and n parents if no hits).
- when opening a screen, go into it. then we go out. the tree looks the same, but the opened screen has a green color (it is active). 
- should lazy load the new screens: only create them once clicked.
- currently i think there is a bug in which the screen not saved when going back (it closes i think).

Also, each screen in the tui when using the WS feature should actually be two screens (behind the scenes) at the same location, but split in a 60%/40% two-pane view. so that we have two screens for the same location .And a hotkey to swap between which vertically seperated pane we are working on.
