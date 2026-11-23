# MCP Raptor Player 1.1.0

Changes since 1.0.2-rc1.

## New features

### ON AIR and EDITOR modes
- New mode button, always visible next to the playing song: gray **EDITOR** (everything allowed) or red **ON AIR**. The chosen mode is remembered.
- ON AIR protects the sequence going on air:
  - Allowed: Stop, Next with the fader, and adding or removing songs one at a time.
  - Blocked: seeking, Previous, jumping to another track, shuffling or sorting, clearing or replacing the playlist, and moving or removing several tracks at once.
- Stopping while ON AIR, or clicking the red button, asks whether to switch back to EDITOR.
- MCP commands follow the same rules and answer with the reason when something is blocked:
  - `controlar_reproduccion` with `detener` stays ON AIR unless `salir_del_aire: true` is passed.
  - `estado_del_reproductor` reports the current mode.

### "Played" column: start times and estimates
- New **Played** column in the playlist, between Duration and Type:
  - for songs that already played, the date and time they last **started** playing (now recorded at the start, not at the end);
  - for the songs after the one playing, the **estimated** start time in italics, taking the mixes into account and updated every second;
  - the tooltip shows how long ago it played, or how long until it starts.
- The last played time can be edited or cleared in the editor's **Data** tab. Future dates are rejected.
- The Catalog shows the full date and time, with seconds, instead of only the day.

### Time zone and date format
- All dates and times follow the time zone of the TTH locality: the clock, the TTH voice, the Played column, the catalog, the details panel and the `played:` search.
- The locality can only be changed in EDITOR mode, from the window or through MCP. Changing it recalculates every time on screen.
- New **Window › Date format** menu, alongside the language menu: dd/mm/yyyy, mm/dd/yyyy or yyyy-mm-dd. It applies everywhere dates are shown or typed:
  - the `played:` search reads dates in the chosen order, including short years like `03/11/26`;
  - the **Next airing** column of the schedule now includes the year.
- Saved files and logs stay in ISO 8601.

### Cover art and internet data
- Cover art in the details panel and the catalog. It comes from the file's tags, then from an image in the song's folder or its album folder, then optionally from the internet (Deezer, iTunes or MusicBrainz with the Cover Art Archive). Hovering shows it enlarged, with its source.
- Without cover art, the panel shows the Raptor "R" in the color of the item type.
- **Complete from the internet** now also uses Deezer and iTunes after MusicBrainz. Matching is strict, compilations are ignored, and edition notes are stripped from album names.
- The catalog remembers which fields you corrected by hand, so updates from tags or the internet don't overwrite them by default.
- Long catalog tasks (internet lookups and cover art from 30 songs, tag rereads from 300) estimate how long they will take and ask before starting.

### Main window
- Large "now playing" line: play icon, type icon in its stripe color, number, title and artist, left-aligned. ON AIR alternates it with the next song, marked with a double-play icon.
- The playing song is shown in cyan and the next one in the normal text color.
- All checkboxes in the main window and effects panel became icon-and-text toggle buttons, green when on. **Free mode** shows an open or closed padlock.
- The playback options moved next to the volume and fader and were renamed **Repeat**, **SmartMix** and **Auto-level**. When space is short they shrink to icons, and the tooltip keeps the name.
- The **Ranking** column is now **Rate**.
- "Waiting" effect notices show the name without the file extension, with tenths of a second.
- The progress bar shows the overlay, outro and mix marks in both modes.

### Playback and breaks
- Commercials, Advertising and Announcements always play at maximum volume without distortion: true peak raised to −1 dBTP, up to +6 dB, never lowered.
- A song's pinned effect, or the TTH voice over its music bed, now waits for its window separately and no longer replaces an effect waiting on the effects board.
- If an advertising order is loaded or changed after its break was already queued, its spots are added to that break if it hasn't played yet, or carried over to the next possible break.
- Effects pages can be renamed: right-click or double-click the page name. MCP `hojas_de_efectos` can also name and rename pages.

### MCP
- New `guardar_lista` tool: saves the playlist as JSON or M3U, with an optional date and time prefix, and never overwrites without being asked.
- New `localidad_del_clima` tool to view or change the weather location.
- The connector now shows the Raptor icon in MCP clients.

## Fixes
- After removing tracks, the selector stays in the same place instead of jumping to the playing song. With Scroll on, the playlist stays put.
- **Clear** now asks for confirmation.
- In the **Update data** context menu, "Complete from the internet" now comes before "Reread the tags".
- Closing the main window now closes every child window first: editor, catalog, orders and the rest.
- The bottom buttons of the Advertising orders window were cut off.
- The large line had extra vertical space and a misaligned artist.
- Several texts showed in Spanish with the English interface, including "Saved", the editor's help line, "inactive" and "(in …)".

## Documentation
- The user manual (Spanish and English, HTML and PDF) and its screenshots were updated for all of the above.
