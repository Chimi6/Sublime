-- Builds the Pages fixture set on a Mac with Pages installed.
--
--   osascript scripts/pages-fixtures/make-fixtures.applescript "$(pwd)"
--
-- For every Word document in tests/fixtures/pages/sources/, Pages opens it,
-- saves it as a .pages document, and exports Apple's own Word, PDF, and
-- plain-text renderings as references. It then builds one document
-- natively through the scripting dictionary. The first run asks for
-- automation permission; allow it. Errors for one document are reported
-- and the script moves on.

on run argv
	if (count of argv) < 1 then error "usage: make-fixtures.applescript <repo root>"
	set repoRoot to item 1 of argv
	set sourcesDir to repoRoot & "/tests/fixtures/pages/sources"
	set fixturesDir to repoRoot & "/tests/fixtures/pages"
	set referenceDir to fixturesDir & "/reference"
	do shell script "mkdir -p " & quoted form of referenceDir
	set sourceNames to paragraphs of (do shell script "ls " & quoted form of sourcesDir & " | grep '\\.docx$'")
	set report to {}
	tell application "Pages"
		activate
		repeat with sourceName in sourceNames
			set baseName to text 1 thru -6 of sourceName
			try
				my convertOne(sourcesDir & "/" & sourceName, fixturesDir, referenceDir, baseName)
				set end of report to baseName & ": ok"
			on error message
				set end of report to baseName & ": FAILED " & message
			end try
		end repeat
		try
			my buildNative(sourcesDir, fixturesDir, referenceDir)
			set end of report to "native-scripted: ok"
		on error message
			set end of report to "native-scripted: FAILED " & message
		end try
		try
			my buildNativeObjects(sourcesDir, fixturesDir, referenceDir)
			set end of report to "native-objects: ok"
		on error message
			set end of report to "native-objects: FAILED " & message
		end try
	end tell
	set AppleScript's text item delimiters to linefeed
	return report as text
end run

-- Opens a Word document, saves it as Pages, exports the references.
on convertOne(sourcePath, fixturesDir, referenceDir, baseName)
	tell application "Pages"
		with timeout of 600 seconds
			-- Start from a clean slate so front document is unambiguous.
			repeat while (count of documents) > 0
				close every document saving no
				delay 0.3
			end repeat
		end timeout
	end tell
	-- Open through LaunchServices rather than the Pages `open` command: the
	-- scripting command is flaky from inside a handler (it can silently fail
	-- to open a document, notably ones with embedded images), whereas
	-- `open -a` is reliable. Poll for the document, then act on it.
	do shell script "open -a Pages " & quoted form of sourcePath
	tell application "Pages"
		with timeout of 600 seconds
			set waited to 0
			repeat until (count of documents) > 0
				delay 0.3
				set waited to waited + 0.3
				if waited > 120 then error "document did not open within 120s"
			end repeat
			delay 1
			set theDocument to front document
			my saveAndExport(theDocument, fixturesDir, referenceDir, baseName)
		end timeout
	end tell
end convertOne

on saveAndExport(theDocument, fixturesDir, referenceDir, baseName)
	tell application "Pages"
		with timeout of 600 seconds
			set pagesPath to fixturesDir & "/" & baseName & ".pages"
			do shell script "rm -rf " & quoted form of pagesPath
			save theDocument in (POSIX file pagesPath)
			delay 1
			export theDocument to (POSIX file (referenceDir & "/" & baseName & ".docx")) as Microsoft Word
			export theDocument to (POSIX file (referenceDir & "/" & baseName & ".pdf")) as PDF
			export theDocument to (POSIX file (referenceDir & "/" & baseName & ".txt")) as unformatted text
			close theDocument saving no
		end timeout
	end tell
end saveAndExport

-- A document authored through the scripting dictionary rather than
-- imported, so the fixture set has one file whose objects Pages created
-- from scratch. The dictionary cannot apply named styles or lists; those
-- come from the imported documents.
on buildNative(sourcesDir, fixturesDir, referenceDir)
	tell application "Pages"
		set theDocument to make new document with properties {document template:template "Blank"}
		delay 1
		tell theDocument
			set body text to "Native Scripted Document" & return & "This paragraph was written by AppleScript, with a font, a size, and a color set on ranges." & return & "A second paragraph in a different font." & return & "The last paragraph."
			tell body text
				set properties of paragraph 1 to {font:"Helvetica Neue Bold", size:28, color:{0, 0, 0}}
				set properties of paragraph 2 to {font:"Helvetica Neue", size:12, color:{13107, 13107, 13107}}
				set properties of paragraph 3 to {font:"Times New Roman Italic", size:14, color:{52428, 0, 0}}
				set properties of paragraph 4 to {font:"Courier New", size:11}
			end tell
		end tell
		-- Tables, images, and text items are elements of a page (or section),
		-- not of the document itself; making them on `theDocument` fails.
		try
			tell page 1 of theDocument
				set theTable to make new table with properties {row count:3, column count:2, header row count:1}
				tell theTable
					set value of cell "A1" to "Header A"
					set value of cell "B1" to "Header B"
					set value of cell "A2" to "one"
					set value of cell "B2" to "1"
					set value of cell "A3" to "two"
					set value of cell "B3" to "2"
				end tell
			end tell
		end try
		try
			tell page 1 of theDocument
				make new image with properties {file:(POSIX file (sourcesDir & "/image1.png"))}
			end tell
		end try
		try
			tell page 1 of theDocument
				make new text item with properties {object text:"A floating text box made by script.", height:120, width:240, position:{72, 500}}
			end tell
		end try
		my saveAndExport(theDocument, fixturesDir, referenceDir, "native-scripted")
	end tell
end buildNative

-- A second scripted document holding drawn objects Pages creates natively:
-- a shape with text, a straight line, and a chart with data. Grouping is not
-- in Pages' scripting dictionary (there is no `group` command), so a group is
-- not included; add one by hand if the map needs it.
on buildNativeObjects(sourcesDir, fixturesDir, referenceDir)
	tell application "Pages"
		set theDocument to make new document with properties {document template:template "Blank"}
		delay 1
		tell theDocument
			set body text to "Native Drawn Objects" & return & "Shapes, a line, and a chart, each authored by AppleScript."
		end tell
		try
			tell page 1 of theDocument
				make new shape with properties {object text:"A shape with text.", position:{72, 140}, width:220, height:110}
			end tell
		end try
		try
			tell page 1 of theDocument
				make new line with properties {start point:{72, 300}, end point:{400, 300}}
			end tell
		end try
		try
			tell page 1 of theDocument
				make new chart with data {{10, 20, 30}, {15, 25, 35}} with properties {position:{72, 360}}
			end tell
		end try
		my saveAndExport(theDocument, fixturesDir, referenceDir, "native-objects")
	end tell
end buildNativeObjects
