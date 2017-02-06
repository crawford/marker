# Marker #

This is a tool for finding issues in [CommonMark][commonmark] documentation.
Right now, it only identifies broken links (malformed URLs, non-existent paths,
etc.).

## Installing Marker ##

Marker can be installed using [cargo][cargo]:

```
cargo install marker
```

## Running Marker ##

When run without any arguments, Marker will search the current working
directory and its descendants for CommonMark documents (everything with a file
extension of `.md`). The `--root` flag can also be provided to change the
search location.

For example, given the following document in the current working directory:

```markdown
This is a [broken reference].
This [won't load](http://www.acrawford.com/404).
[I cannot type](http:://example.com)
[This file](not_here.md) doesn't exist.
This is an [absolute path](/root.md).
```

When Marker is run, the following is output and the program exits with a
non-zero exit status:

```
Found broken reference ([broken reference]) in ./example.md
Found broken url       (404 -> http://www.acrawford.com/404) in ./example.md: 404 Not Found
Found malformed URL    (malformed url -> http:://example.com) in ./example.md: empty host
Found broken path      (bad path -> not_here.md) in ./example.md
Found absolute path    (absolute path -> /root.md) in ./example.md
```

[cargo]: http://doc.crates.io/guide.html
[commonmark]: http://commonmark.org/
