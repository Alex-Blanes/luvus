#!/bin/sh
# Node right-click "Present as slides": open the presenter pane. luvus injects
# LUVUS_BIN_PATH so we call the exact binary that launched us.
exec "${LUVUS_BIN_PATH:-luvus}" module pane open example.deck present
