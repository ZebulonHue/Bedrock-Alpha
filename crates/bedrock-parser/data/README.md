# Vendored data

## `bedrock_blocks_b2j.json`

Bedrock Edition block states mapped to their Java Edition equivalents, for
Minecraft 1.26.20. Without it a Bedrock save cannot be read at all.

Taken from [PrismarineJS/minecraft-data][1] (`data/bedrock/1.26.20/blocksB2J.json`),
which is MIT licensed. Minified and key-sorted on the way in; the contents are
otherwise unchanged.

It lives here rather than being read from `third_party/` because the parser
embeds it at compile time. Pointing that at a sibling clone meant the
repository could not be built by anyone who had not also cloned a 642 MB data
repo next to it — which is to say, by anyone but us.

[1]: https://github.com/PrismarineJS/minecraft-data
