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
	end tell
	set AppleScript's text item delimiters to linefeed
	return report as text
end run

-- Opens a Word document, saves it as Pages, exports the references.
on convertOne(sourcePath, fixturesDir, referenceDir, baseName)
	tell application "Pages"
		set theDocument to open (POSIX file sourcePath)
		delay 1
		my saveAndExport(theDocument, fixturesDir, referenceDir, baseName)
	end tell
end convertOne

on saveAndExport(theDocument, fixturesDir, referenceDir, baseName)
	tell application "Pages"
		set pagesPath to fixturesDir & "/" & baseName & ".pages"
		do shell script "rm -rf " & quoted form of pagesPath
		save theDocument in (POSIX file pagesPath)
		delay 1
		export theDocument to (POSIX file (referenceDir & "/" & baseName & ".docx")) as Microsoft Word
		export theDocument to (POSIX file (referenceDir & "/" & baseName & ".pdf")) as PDF
		export theDocument to (POSIX file (referenceDir & "/" & baseName & ".txt")) as unformatted text
		close theDocument saving no
	end tell
end saveAndExport

-- A document authored through the scripting dictionary rather than
-- imported, so the fixture set has one file whose objects Pages created
-- from scratch. The dictionary cannot apply named styles or lists; those
-- come from the imported documents.
on buildNative(sourcesDir, fixturesDir, referenceDir)
	tell application "Pages"
		set theDocument to make new document with properties {document template:template "Blank"}
		tell theDocument
			set body text to "Native Scripted Document" & return & "This paragraph was written by AppleScript, with a font, a size, and a color set on ranges." & return & "A second paragraph in a different font." & return & "The last paragraph."
			tell body text
				set properties of paragraph 1 to {font:"Helvetica Neue Bold", size:28, color:{0, 0, 0}}
				set properties of paragraph 2 to {font:"Helvetica Neue", size:12, color:{13107, 13107, 13107}}
				set properties of paragraph 3 to {font:"Times New Roman Italic", size:14, color:{52428, 0, 0}}
				set properties of paragraph 4 to {font:"Courier New", size:11}
			end tell
		end tell
		try
			tell theDocument
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
			tell theDocument
				make new image with properties {file:(POSIX file (sourcesDir & "/image1.png"))}
			end tell
		end try
		try
			tell theDocument
				set theTextItem to make new text item with properties {height:120, width:240, position:{72, 500}}
				set object text of theTextItem to "A floating text box made by script."
			end tell
		end try
		my saveAndExport(theDocument, fixturesDir, referenceDir, "native-scripted")
	end tell
end buildNative
