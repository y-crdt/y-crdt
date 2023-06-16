# Single user editing traces

This folder contains a set of simple, sequential text editing traces from real editing sessions. In each of these recorded traces, every editing event a user made in the document was recorded.

Most of these editing traces come from a single user typing. This folder also contains linearized (flattened) copies of the concurrent editing traces, where all editing positions have been transformed such that they can be applied in order into a text document to produce the correct result.

These editing traces used to live in the root directory of this repository.

These traces are useful to test the performance of CRDT implementations and rope libraries and things like that, which edit text.


## Data format

These editing histories all have a simple, common format:

```javascript
{
    startContent: '', // All traces start with empty documents
    endContent: '...', // Content after all patches have been applied

    txns: [{ // Each transaction contains 1 or more patches
        time: '2021-04-19T06:06:58.000Z', // <-- ISO timestamp of change
        patches: [ //
            [2206,0,"r"] // Patches like [position, numDeleted, inserted content]
            [2204,1,""],
            /// ...
        ]
    }, // ...
    ]
}
```

For each file you can start with `startContent` (always the empty string), then apply all patches in each transaction in the file. You should end up with `endContent`.

Each patch is an array with 3 fields:

1. Position in the document. This number is specified in *unicode codepoints*. See below for notes on unicode length.
2. Number of deleted characters at this position
3. Inserted characters at this position, or an empty string.

This matches the semantics of [`Array.splice`](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Array/splice).

Every patch will always either delete or insert something. Sometimes both.

When transactions contain multiple patches, the locations are in descending order so the patches are independent and can easily be applied sequentially.

Every transaction must have at least one patch.

The patches can be applied with this code:

```javascript
const {
  startContent,
  endContent,
  txns
} = JSON.parse(fs.readFileSync(filename, 'utf-8'))

let content = startContent

for (const {patches} in txns) {
  for (const [pos, delHere, insContent] of patches) {
    const before = content.slice(0, pos)
    const after = content.slice(pos + delHere)
    content = before + insContent + after
  }
}

assert(content === endContent)
```

Files are gzipped. Uncompress them on mac / linux using `gunzip *.gz`.

See check.js for a more complete example reading these files in javascript.


## Data sets

#### rustcode and sveltecomponent

These data sets were recorded by me (Joseph Gentle) using the [vscode tracker](https://github.com/josephg/vscode-tracker/). The change sets include multi-cursor edits and refactoring operations.

The `rustcode.json.gz` change set contains many of the edits to [this rust sourcecode file](https://github.com/josephg/skiplistrs/blob/140fe17f484daa2bf4e32983f6a4ce60020eee1a/src/skiplist.rs). Unfortunately most of the code was written before I started tracing edits.

And `sveltecomponent.json.gz` contains edits from [this svelte component](https://github.com/josephg/glassbeadtimer/blob/c3d8e14e2abc998a328cdabbd559c4db10b42e5b/src/App.svelte), which contains the UI for [this multiplayer game timer](https://glassbead.seph.codes/). This dataset is pure ASCII.

Timestamps for these traces are limited to second resolution for privacy.

These data sets are provided under the [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) license.


#### automerge-paper.json.gz

This data set is copied from the [automerge-perf](https://github.com/automerge/automerge-perf/) repository, prepared by [Martin Kleppmann](https://martin.kleppmann.com/). It contains the editing trace from writing [this LaTeX paper](https://arxiv.org/abs/1608.03960). The trace contains 182,315 inserts and 77,463 deletes.

Due to the editor in which the paper was written, each txn contains exactly 1 patch, which is always either a single character insert or single character delete. Also there are no timestamps, so it looks like every edit happened at the same moment. This dataset is pure ASCII.


#### seph-blog1.json.gz

This contains the character by character editing trace while writing this blog post about CRDTs: [https://josephg.com/blog/crdts-go-brrr/](https://josephg.com/blog/crdts-go-brrr/). The post was written in markdown. This trace is similar to automerge-paper, since they both edit text - but it is about half the size and it has more variety in edit lengths.

At one point while writing this post, I thought inlining the images in SVG might be a good idea - so there's a single large insert of 4kb of content of SVG source (followed by me hitting undo!)

The final document length is 56769 characters.

This data set is provided under the [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) license.



#### friendsforever_flat.json.gz

This is the same as the friendsforever trace from the concurrent editing traces, except all patch positions have been rewritten such that the whole trace can be applied in sequence. (In OT parlance, the editing trace has been fully linearized / transformed).

Note this editing trace was actually written by 2 users typing concurrently, so the editing positions "ping pong" around a lot more than they will in other traces in this folder.

This dataset is pure ASCII.


## Unicode lengths. Oh my!

**tldr:** If you don't want to think about unicode while benchmarking, use the variants in the `ascii_only/` subdirectory. That directory contains copies of the traces where all non-ascii characters are replaced with underscores.

---

There's lots of different ways to count the "length" of a string. These are:

- Number of UTF8 bytes
- UCS2 length
- Number of unicode codepoints
- Number of grapheme clusters (printed characters)

To make matters worse, different languages pick different defaults:

- Some languages natively measure strings by counting UTF8 bytes, like Rust and Go
- Some languages natively measure strings by counting the UCS2 length, like Javascript, C#, Java, Obj-C and others

This is a problem, because if you do the naive thing in your language - and write code like this:

```javascript
for (const [pos, delHere, insContent] of patches) {
  content = content.slice(0, pos) + insContent + content.slice(pos + delHere)
}
```

Then `string.slice()` needs to interpret the positions correctly.

The data sets included here specify positions using unicode codepoints - which isn't the native encoding for *any* language. I like counting codepoints because its language-agnostic and it makes more sense for CRDTs.

To process these editing traces correctly, we can think about 3 "types" of characters in a document:

1. ASCII characters. These have a "size" of 1 in all measurements, in all languages. Simple and easy. The `automerge-paper` and `sveltecomponent` datasets only contain ASCII characters.
2. Codepoints from `U+0080` to `U+FFFF`. These have a size 1 in UCS2 languages (javascript, java, C#, ...). But the number of UTF8 bytes is more than 1 - so in Rust and Go they'll appear "longer". The `rustcode` and `seph-blog1` datasets contain some characters in this unicode range.
3. Codepoints from `U+10000` and up. The UCS2 length and UTF8 byte length will be more than 1 in all languages. None of the datasets contain characters in this unicode range.

Since I'm counting positions & deleted lengths using unicode codepoints, all of these 3 categories are counted as "1 character" for the purpose of edit positions and delete lengths. Because none of these data sets (so far) use anything in group 3, you can naively use `string.slice()` in javascript, and equivalents in Java, C#, etc.

**But** if your code expects offsets to be specified by counting UTF8 bytes (eg in Rust or Go), you can't use the lengths in some data sets directly. You have 2 choices:

1. Convert all positions & deleted lengths to byte offsets. ([Here's some rust code that does that](../../../../editing-traces/rust/src/lib.rs#L44-L74)). Or
2. Use the dataset copies in the `ascii_only/` directory. These have non-ASCII characters replaced with underscores (`'_'`).

Using the `ascii_only` variants should be fine for most benchmarks - there's only a few sporadic non-ASCII characters anyway. But there is definitely an English-speaking bias to this. I'd really like to have some data sets where the whole editing trace is in another language - especially a non-european language (where almost *none* of the characters are ASCII). If you speak another language, please contribute!
