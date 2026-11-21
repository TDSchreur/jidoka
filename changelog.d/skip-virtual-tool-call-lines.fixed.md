## Fixed

- A run's Claude Code usage is no longer marked partial because of virtual tool-call lines (`isVirtual`, model `<synthetic>`, all counts zero) that newer Claude Code versions write for tools called from a script tool. They are skipped like usage-limit messages.
