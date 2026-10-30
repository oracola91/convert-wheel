// Icons for the Tools and Actions wheels: the inside of 24x24 icons from Lucide 1.54.0
// (https://lucide.dev), drawn with a 2 px round stroke. Licence in icons-LICENSE.txt:
// ISC, and MIT for the icons that come from Feather.

const ICONS = {
  "tool:compress": "<path d=\"m14 10 7-7\" /><path d=\"M20 10h-6V4\" /><path d=\"m3 21 7-7\" /><path d=\"M4 14h6v6\" />", // minimize-2
  "tool:half": "<path d=\"M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7\" /><path d=\"M14 15H9v-5\" /><path d=\"M16 3h5v5\" /><path d=\"M21 3 9 15\" />", // scaling
  "tool:clean": "<path d=\"M21 21H8a2 2 0 0 1-1.42-.587l-3.994-3.999a2 2 0 0 1 0-2.828l10-10a2 2 0 0 1 2.829 0l5.999 6a2 2 0 0 1 0 2.828L12.834 21\" /><path d=\"m5.082 11.09 8.828 8.828\" />", // eraser
  "tool:rotate": "<path d=\"M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8\" /><path d=\"M21 3v5h-5\" />", // rotate-cw
  "tool:crop": "<path d=\"M6 2v14a2 2 0 0 0 2 2h14\" /><path d=\"M18 22V8a2 2 0 0 0-2-2H2\" />", // crop
  "tool:adjust": "<circle cx=\"12\" cy=\"12\" r=\"4\" /><path d=\"M12 3v1\" /><path d=\"M12 20v1\" /><path d=\"M3 12h1\" /><path d=\"M20 12h1\" /><path d=\"m18.364 5.636-.707.707\" /><path d=\"m6.343 17.657-.707.707\" /><path d=\"m5.636 5.636.707.707\" /><path d=\"m17.657 17.657.707.707\" />", // sun-medium
  "tool:markup": "<path d=\"M13 21h8\" /><path d=\"M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z\" />", // pen-line
  "tool:trim": "<circle cx=\"6\" cy=\"6\" r=\"3\" /><path d=\"M8.12 8.12 12 12\" /><path d=\"M20 4 8.12 15.88\" /><circle cx=\"6\" cy=\"18\" r=\"3\" /><path d=\"M14.8 14.8 20 20\" />", // scissors
  "tool:bleep": "<path d=\"M11 4.702a.7.7 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.7.7 0 0 0 11 19.298z\" /><path d=\"m16.5 14.5 5-5\" /><path d=\"m16.5 9.5 5 5\" />", // volume-x
  "tool:speed": "<path d=\"m12 14 4-4\" /><path d=\"M3.34 19a10 10 0 1 1 17.32 0\" />", // gauge
  "tool:parts": "<path d=\"M12 2v20\" /><path d=\"M16 3h3a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-3\" /><path d=\"M8 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3\" />", // square-split-horizontal
  "tool:join": "<path d=\"M14 3a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1\" /><path d=\"M19 3a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1\" /><path d=\"m7 15 3 3\" /><path d=\"m7 21 3-3H5a2 2 0 0 1-2-2v-2\" /><rect x=\"14\" y=\"14\" width=\"7\" height=\"7\" rx=\"1\" /><rect x=\"3\" y=\"3\" width=\"7\" height=\"7\" rx=\"1\" />", // combine
  "tool:target": "<circle cx=\"12\" cy=\"12\" r=\"10\" /><circle cx=\"12\" cy=\"12\" r=\"6\" /><circle cx=\"12\" cy=\"12\" r=\"2\" />", // target
  "tool:qr": "<rect width=\"5\" height=\"5\" x=\"3\" y=\"3\" rx=\"1\" /><rect width=\"5\" height=\"5\" x=\"16\" y=\"3\" rx=\"1\" /><rect width=\"5\" height=\"5\" x=\"3\" y=\"16\" rx=\"1\" /><path d=\"M21 16h-3a2 2 0 0 0-2 2v3\" /><path d=\"M21 21v.01\" /><path d=\"M12 7v3a2 2 0 0 1-2 2H7\" /><path d=\"M3 12h.01\" /><path d=\"M12 3h.01\" /><path d=\"M12 16v.01\" /><path d=\"M16 12h1\" /><path d=\"M21 12v.01\" /><path d=\"M12 21v-1\" />", // qr-code
  "tool:onepdf": "<path d=\"M11 21a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1\" /><path d=\"M16 16a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1\" /><path d=\"M21 6a2 2 0 0 0-.586-1.414l-2-2A2 2 0 0 0 17 2h-3a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h6a1 1 0 0 0 1-1z\" />", // file-stack
  "tool:collage": "<rect width=\"7\" height=\"7\" x=\"3\" y=\"3\" rx=\"1\" /><rect width=\"7\" height=\"7\" x=\"14\" y=\"3\" rx=\"1\" /><rect width=\"7\" height=\"7\" x=\"14\" y=\"14\" rx=\"1\" /><rect width=\"7\" height=\"7\" x=\"3\" y=\"14\" rx=\"1\" />", // layout-grid
  "tool:merge": "<path d=\"m8 6 4-4 4 4\" /><path d=\"M12 2v10.3a4 4 0 0 1-1.172 2.872L4 22\" /><path d=\"m20 22-5-5\" />", // merge
  "tool:split": "<path d=\"M16 3h5v5\" /><path d=\"M8 3H3v5\" /><path d=\"M12 22v-8.3a4 4 0 0 0-1.172-2.872L3 3\" /><path d=\"m15 9 6-6\" />", // split
  "tool:order": "<path d=\"m3 16 4 4 4-4\" /><path d=\"M7 20V4\" /><path d=\"m21 8-4-4-4 4\" /><path d=\"M17 4v16\" />", // arrow-down-up
  "tool:info": "<circle cx=\"12\" cy=\"12\" r=\"10\" /><path d=\"M12 16v-4\" /><path d=\"M12 8h.01\" />", // info
  "tool:smaller": "<path d=\"M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z\" /><path d=\"M14 2v5a1 1 0 0 0 1 1h5\" /><path d=\"M12 18v-6\" /><path d=\"m9 15 3 3 3-3\" />", // file-down
  "tool:still": "<rect width=\"18\" height=\"18\" x=\"3\" y=\"3\" rx=\"2\" ry=\"2\" /><circle cx=\"9\" cy=\"9\" r=\"2\" /><path d=\"m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21\" />", // image
  "tool:loudness": "<path d=\"M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z\" /><path d=\"M16 9a5 5 0 0 1 0 6\" /><path d=\"M19.364 18.364a9 9 0 0 0 0-12.728\" />", // volume-2
  "tool:mono": "<path d=\"M2 10v3\" /><path d=\"M6 6v11\" /><path d=\"M10 3v18\" /><path d=\"M14 8v7\" /><path d=\"M18 5v13\" /><path d=\"M22 10v3\" />", // audio-lines
  "act:path": "<rect width=\"14\" height=\"14\" x=\"8\" y=\"8\" rx=\"2\" ry=\"2\" /><path d=\"M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2\" />", // copy
  "act:show": "<path d=\"M10.7 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v4.1\" /><path d=\"m21 21-1.9-1.9\" /><circle cx=\"17\" cy=\"17\" r=\"3\" />", // folder-search
  "act:copyto": "<line x1=\"15\" x2=\"15\" y1=\"12\" y2=\"18\" /><line x1=\"12\" x2=\"18\" y1=\"15\" y2=\"15\" /><rect width=\"14\" height=\"14\" x=\"8\" y=\"8\" rx=\"2\" ry=\"2\" /><path d=\"M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2\" />", // copy-plus
  "act:moveto": "<path d=\"M2 9V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-1\" /><path d=\"M2 13h10\" /><path d=\"m9 16 3-3-3-3\" />", // folder-input
  "act:terminal": "<path d=\"M12 19h8\" /><path d=\"m4 17 6-6-6-6\" />", // terminal
  "act:airdrop": "<path d=\"M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z\" /><path d=\"m21.854 2.147-10.94 10.939\" />", // send
  "act:zip": "<rect width=\"20\" height=\"5\" x=\"2\" y=\"3\" rx=\"1\" /><path d=\"M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8\" /><path d=\"M10 12h4\" />", // archive
  "act:tgz": "<rect width=\"20\" height=\"5\" x=\"2\" y=\"3\" rx=\"1\" /><path d=\"M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8\" /><path d=\"M10 12h4\" />", // archive
  "act:tar": "<rect width=\"20\" height=\"5\" x=\"2\" y=\"3\" rx=\"1\" /><path d=\"M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8\" /><path d=\"M10 12h4\" />", // archive
  "act:unpack": "<path d=\"M12 22v-9\" /><path d=\"M15.17 2.21a1.67 1.67 0 0 1 1.63 0L21 4.57a1.93 1.93 0 0 1 0 3.36L8.82 14.79a1.655 1.655 0 0 1-1.64 0L3 12.43a1.93 1.93 0 0 1 0-3.36z\" /><path d=\"M20 13v3.87a2.06 2.06 0 0 1-1.11 1.83l-6 3.08a1.93 1.93 0 0 1-1.78 0l-6-3.08A2.06 2.06 0 0 1 4 16.87V13\" /><path d=\"M21 12.43a1.93 1.93 0 0 0 0-3.36L8.83 2.2a1.64 1.64 0 0 0-1.63 0L3 4.57a1.93 1.93 0 0 0 0 3.36l12.18 6.86a1.636 1.636 0 0 0 1.63 0z\" />", // package-open
};
