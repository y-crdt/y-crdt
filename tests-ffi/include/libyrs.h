/**
 * The MIT License (MIT)
 *
 *  Copyright (c) 2020
 *    - Bartosz Sypytkowski <b.sypytkowski@gmail.com>
 *    - Kevin Jahns <kevin.jahns@pm.me>.
 *
 *  Permission is hereby granted, free of charge, to any person obtaining a copy
 *  of this software and associated documentation files (the "Software"), to deal
 *  in the Software without restriction, including without limitation the rights
 *  to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 *  copies of the Software, and to permit persons to whom the Software is
 *  furnished to do so, subject to the following conditions:
 *
 *  The above copyright notice and this permission notice shall be included in all
 *  copies or substantial portions of the Software.
 *
 *  THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 *  IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 *  FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 *  AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 *  LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 *  OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 *  SOFTWARE.
 */

#ifndef YRS_FFI_H
#define YRS_FFI_H

/**
 * A Yrs document type. Documents are most important units of collaborative resources management.
 * All shared collections live within a scope of their corresponding documents. All updates are
 * generated on per document basis (rather than individual shared type). All operations on shared
 * collections happen via `YTransaction`, which lifetime is also bound to a document.
 *
 * Document manages so called root types, which are top-level shared types definitions (as opposed
 * to recursively nested types).
 */
typedef struct YDoc {} YDoc;

/**
 * A common shared data type. All Yrs instances can be refered to using this data type (use
 * `ytype_kind` function if a specific type needs to be determined). Branch pointers are passed
 * over type-specific functions like `ytext_insert`, `yarray_insert` or `ymap_insert` to perform
 * a specific shared type operations.
 *
 * Using write methods of different shared types (eg. `ytext_insert` and `yarray_insert`) over
 * the same branch may result in undefined behavior.
 */
typedef struct Branch {} Branch;

/**
 * Transaction is one of the core types in Yrs. All operations that need to touch a document's
 * contents (a.k.a. block store), need to be executed in scope of a transaction.
 */
typedef struct YTransaction {} YTransaction;

/**
 * Iterator structure used by shared array data type.
 */
typedef struct YArrayIter {} YArrayIter;

/**
 * Iterator structure used by shared map data type. Map iterators are unordered - there's no
 * specific order in which map entries will be returned during consecutive iterator calls.
 */
typedef struct YMapIter {} YMapIter;

/**
 * Iterator structure used by XML nodes (elements and text) to iterate over node's attributes.
 * Attribute iterators are unordered - there's no specific order in which map entries will be
 * returned during consecutive iterator calls.
 */
typedef struct YXmlAttrIter {} YXmlAttrIter;

/**
 * Iterator used to traverse over the complex nested tree structure of a XML node. XML node
 * iterator walks only over `YXmlElement` and `YXmlText` nodes. It does so in ordered manner (using
 * the order in which children are ordered within their parent nodes) and using **depth-first**
 * traverse.
 */
typedef struct YXmlTreeWalker {} YXmlTreeWalker;


#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * Flag used by `YInput` and `YOutput` to tag boolean values.
 */
#define Y_JSON_BOOL -8

/**
 * Flag used by `YInput` and `YOutput` to tag floating point numbers.
 */
#define Y_JSON_NUM -7

/**
 * Flag used by `YInput` and `YOutput` to tag 64-bit integer numbers.
 */
#define Y_JSON_INT -6

/**
 * Flag used by `YInput` and `YOutput` to tag strings.
 */
#define Y_JSON_STR -5

/**
 * Flag used by `YInput` and `YOutput` to tag binary content.
 */
#define Y_JSON_BUF -4

/**
 * Flag used by `YInput` and `YOutput` to tag embedded JSON-like arrays of values,
 * which themselves are `YInput` and `YOutput` instances respectively.
 */
#define Y_JSON_ARR -3

/**
 * Flag used by `YInput` and `YOutput` to tag embedded JSON-like maps of key-value pairs,
 * where keys are strings and v
 */
#define Y_JSON_MAP -2

/**
 * Flag used by `YInput` and `YOutput` to tag JSON-like null values.
 */
#define Y_JSON_NULL -1

/**
 * Flag used by `YInput` and `YOutput` to tag JSON-like undefined values.
 */
#define Y_JSON_UNDEF 0

/**
 * Flag used by `YInput` and `YOutput` to tag content, which is an `YArray` shared type.
 */
#define Y_ARRAY 1

/**
 * Flag used by `YInput` and `YOutput` to tag content, which is an `YMap` shared type.
 */
#define Y_MAP 2

/**
 * Flag used by `YInput` and `YOutput` to tag content, which is an `YText` shared type.
 */
#define Y_TEXT 3

/**
 * Flag used by `YInput` and `YOutput` to tag content, which is an `YXmlElement` shared type.
 */
#define Y_XML_ELEM 4

/**
 * Flag used by `YInput` and `YOutput` to tag content, which is an `YXmlText` shared type.
 */
#define Y_XML_TEXT 5

/**
 * Flag used to mark a truthy boolean numbers.
 */
#define Y_TRUE 1

/**
 * Flag used to mark a falsy boolean numbers.
 */
#define Y_FALSE 0

/**
 * Flag used by `YOptions` to determine, that text operations offsets and length will be counted by
 * the byte number of UTF8-encoded string.
 */
#define Y_OFFSET_BYTES 0

/**
 * Flag used by `YOptions` to determine, that text operations offsets and length will be counted by
 * UTF-16 chars of encoded string.
 */
#define Y_OFFSET_UTF16 1

/**
 * Flag used by `YOptions` to determine, that text operations offsets and length will be counted by
 * by UTF-32 chars of encoded string.
 */
#define Y_OFFSET_UTF32 2

/**
 * Error code: couldn't read data from input stream.
 */
#define ERR_CODE_IO 1

/**
 * Error code: decoded variable integer outside of the expected integer size bounds.
 */
#define ERR_CODE_VAR_INT 2

/**
 * Error code: end of stream found when more data was expected.
 */
#define ERR_CODE_EOS 3

/**
 * Error code: decoded enum tag value was not among known cases.
 */
#define ERR_CODE_UNEXPECTED_VALUE 4

/**
 * Error code: failure when trying to decode JSON content.
 */
#define ERR_CODE_INVALID_JSON 5

/**
 * Error code: other error type than the one specified.
 */
#define ERR_CODE_OTHER 6

/**
 * Tag used to identify `YPathSegment` storing a *char parameter.
 */
#define Y_EVENT_PATH_KEY 1

/**
 * Tag used to identify `YPathSegment` storing an int parameter.
 */
#define Y_EVENT_PATH_INDEX 2

/**
 * Tag used to identify `YEventChange` (see: `yevent_delta` function) case, when a new element
 * has been added to an observed collection.
 */
#define Y_EVENT_CHANGE_ADD 1

/**
 * Tag used to identify `YEventChange` (see: `yevent_delta` function) case, when an existing
 * element has been removed from an observed collection.
 */
#define Y_EVENT_CHANGE_DELETE 2

/**
 * Tag used to identify `YEventChange` (see: `yevent_delta` function) case, when no changes have
 * been detected for a particular range of observed collection.
 */
#define Y_EVENT_CHANGE_RETAIN 3

/**
 * Tag used to identify `YEventKeyChange` (see: `yevent_keys` function) case, when a new entry has
 * been inserted into a map component of shared collection.
 */
#define Y_EVENT_KEY_CHANGE_ADD 4

/**
 * Tag used to identify `YEventKeyChange` (see: `yevent_keys` function) case, when an existing
 * entry has been removed from a map component of shared collection.
 */
#define Y_EVENT_KEY_CHANGE_DELETE 5

/**
 * Tag used to identify `YEventKeyChange` (see: `yevent_keys` function) case, when an existing
 * entry has been overridden with a new value within a map component of shared collection.
 */
#define Y_EVENT_KEY_CHANGE_UPDATE 6

/**
 * A Yrs document type. Documents are most important units of collaborative resources management.
 * All shared collections live within a scope of their corresponding documents. All updates are
 * generated on per document basis (rather than individual shared type). All operations on shared
 * collections happen via `YTransaction`, which lifetime is also bound to a document.
 *
 * Document manages so called root types, which are top-level shared types definitions (as opposed
 * to recursively nested types).
 */
typedef YDoc YDoc;

/**
 * A common shared data type. All Yrs instances can be refered to using this data type (use
 * `ytype_kind` function if a specific type needs to be determined). Branch pointers are passed
 * over type-specific functions like `ytext_insert`, `yarray_insert` or `ymap_insert` to perform
 * a specific shared type operations.
 *
 * Using write methods of different shared types (eg. `ytext_insert` and `yarray_insert`) over
 * the same branch may result in undefined behavior.
 */
typedef Branch Branch;

typedef union YOutputContent {
  char flag;
  float num;
  long long integer;
  char *str;
  unsigned char *buf;
  struct YOutput *array;
  struct YMapEntry *map;
  Branch *y_type;
} YOutputContent;

/**
 * An output value cell returned from yrs API methods. It describes a various types of data
 * supported by yrs shared data types.
 *
 * Since `YOutput` instances are always created by calling the corresponding yrs API functions,
 * they eventually should be deallocated using [youtput_destroy] function.
 */
typedef struct YOutput {
  /**
   * Tag describing, which `value` type is being stored by this input cell. Can be one of:
   *
   * - [Y_JSON_BOOL] for boolean flags.
   * - [Y_JSON_NUM] for 64-bit floating point numbers.
   * - [Y_JSON_INT] for 64-bit signed integers.
   * - [Y_JSON_STR] for null-terminated UTF-8 encoded strings.
   * - [Y_JSON_BUF] for embedded binary data.
   * - [Y_JSON_ARR] for arrays of JSON-like values.
   * - [Y_JSON_MAP] for JSON-like objects build from key-value pairs.
   * - [Y_JSON_NULL] for JSON-like null values.
   * - [Y_JSON_UNDEF] for JSON-like undefined values.
   * - [Y_TEXT] for pointers to `YText` data types.
   * - [Y_ARRAY] for pointers to `YArray` data types.
   * - [Y_MAP] for pointers to `YMap` data types.
   * - [Y_XML_ELEM] for pointers to `YXmlElement` data types.
   * - [Y_XML_TEXT] for pointers to `YXmlText` data types.
   */
  int8_t tag;
  /**
   * Length of the contents stored by a current `YOutput` cell.
   *
   * For [Y_JSON_NULL] and [Y_JSON_UNDEF] its equal to `0`.
   *
   * For [Y_JSON_ARR], [Y_JSON_MAP] it describes a number of passed elements.
   *
   * For other types it's always equal to `1`.
   */
  int len;
  /**
   * Union struct which contains a content corresponding to a provided `tag` field.
   */
  union YOutputContent value;
} YOutput;

/**
 * A structure representing single key-value entry of a map output (used by either
 * embedded JSON-like maps or YMaps).
 */
typedef struct YMapEntry {
  /**
   * Null-terminated string representing an entry's key component. Encoded as UTF-8.
   */
  const char *key;
  /**
   * A `YOutput` value representing containing variadic content that can be stored withing map's
   * entry.
   */
  struct YOutput value;
} YMapEntry;

/**
 * A structure representing single attribute of an either `YXmlElement` or `YXmlText` instance.
 * It consists of attribute name and string, both of which are null-terminated UTF-8 strings.
 */
typedef struct YXmlAttr {
  const char *name;
  const char *value;
} YXmlAttr;

/**
 * Configuration object used by `YDoc`.
 */
typedef struct YOptions {
  /**
   * Globally unique 53-bit integer assigned to corresponding document replica as its identifier.
   *
   * If two clients share the same `id` and will perform any updates, it will result in
   * unrecoverable document state corruption. The same thing may happen if the client restored
   * document state from snapshot, that didn't contain all of that clients updates that were sent
   * to other peers.
   */
  unsigned long id;
  /**
   * Encoding used by text editing operations on this document. It's used to compute
   * `YText`/`YXmlText` insertion offsets and text lengths. Either:
   *
   * - `Y_ENCODING_BYTES`
   * - `Y_ENCODING_UTF16`
   * - `Y_ENCODING_UTF32`
   */
  int encoding;
  /**
   * Boolean flag used to determine if deleted blocks should be garbage collected or not
   * during the transaction commits. Setting this value to 0 means GC will be performed.
   */
  int skip_gc;
} YOptions;

/**
 * Struct representing a state of a document. It contains the last seen clocks for blocks submitted
 * per any of the clients collaborating on document updates.
 */
typedef struct YStateVector {
  /**
   * Number of clients. It describes a length of both `client_ids` and `clocks` arrays.
   */
  int entries_count;
  /**
   * Array of unique client identifiers (length is given in `entries_count` field). Each client
   * ID has corresponding clock attached, which can be found in `clocks` field under the same
   * index.
   */
  long long *client_ids;
  /**
   * Array of clocks (length is given in `entries_count` field) known for each client. Each clock
   * has a corresponding client identifier attached, which can be found in `client_ids` field
   * under the same index.
   */
  int *clocks;
} YStateVector;

typedef struct YIdRange {
  int start;
  int end;
} YIdRange;

/**
 * Fixed-length sequence of ID ranges. Each range is a pair of [start, end) values, describing the
 * range of items identified by clock values, that this range refers to.
 */
typedef struct YIdRangeSeq {
  /**
   * Number of ranges stored in this sequence.
   */
  int len;
  /**
   * Array (length is stored in `len` field) or ranges. Each range is a pair of [start, end)
   * values, describing continuous collection of items produced by the same client, identified
   * by clock values, that this range refers to.
   */
  struct YIdRange *seq;
} YIdRangeSeq;

/**
 * Delete set is a map of `(ClientID, Range[])` entries. Length of a map is stored in
 * `entries_count` field. ClientIDs reside under `client_ids` and their corresponding range
 * sequences can be found under the same index of `ranges` field.
 */
typedef struct YDeleteSet {
  /**
   * Number of client identifier entries.
   */
  int entries_count;
  /**
   * Array of unique client identifiers (length is given in `entries_count` field). Each client
   * ID has corresponding sequence of ranges attached, which can be found in `ranges` field under
   * the same index.
   */
  long long *client_ids;
  /**
   * Array of range sequences (length is given in `entries_count` field). Each sequence has
   * a corresponding client ID attached, which can be found in `client_ids` field under
   * the same index.
   */
  struct YIdRangeSeq *ranges;
} YDeleteSet;

/**
 * Event generated for callbacks subscribed using `ydoc_observe_after_transaction`. It contains
 * snapshot of changes made within any committed transaction.
 */
typedef struct YAfterTransactionEvent {
  /**
   * Descriptor of a document state at the moment of creating the transaction.
   */
  struct YStateVector before_state;
  /**
   * Descriptor of a document state at the moment of committing the transaction.
   */
  struct YStateVector after_state;
  /**
   * Information about all items deleted within the scope of a transaction.
   */
  struct YDeleteSet delete_set;
} YAfterTransactionEvent;

/**
 * Transaction is one of the core types in Yrs. All operations that need to touch a document's
 * contents (a.k.a. block store), need to be executed in scope of a transaction.
 */
typedef YTransaction YTransaction;

typedef struct YMapInputData {
  char **keys;
  struct YInput *values;
} YMapInputData;

typedef union YInputContent {
  char flag;
  float num;
  long integer;
  char *str;
  unsigned char *buf;
  struct YInput *values;
  struct YMapInputData map;
} YInputContent;

/**
 * A data structure that is used to pass input values of various types supported by Yrs into a
 * shared document store.
 *
 * `YInput` constructor function don't allocate any resources on their own, neither they take
 * ownership by pointers to memory blocks allocated by user - for this reason once an input cell
 * has been used, its content should be freed by the caller.
 */
typedef struct YInput {
  /**
   * Tag describing, which `value` type is being stored by this input cell. Can be one of:
   *
   * - [Y_JSON_BOOL] for boolean flags.
   * - [Y_JSON_NUM] for 64-bit floating point numbers.
   * - [Y_JSON_INT] for 64-bit signed integers.
   * - [Y_JSON_STR] for null-terminated UTF-8 encoded strings.
   * - [Y_JSON_BUF] for embedded binary data.
   * - [Y_JSON_ARR] for arrays of JSON-like values.
   * - [Y_JSON_MAP] for JSON-like objects build from key-value pairs.
   * - [Y_JSON_NULL] for JSON-like null values.
   * - [Y_JSON_UNDEF] for JSON-like undefined values.
   * - [Y_ARRAY] for cells which contents should be used to initialize a `YArray` shared type.
   * - [Y_MAP] for cells which contents should be used to initialize a `YMap` shared type.
   */
  int8_t tag;
  /**
   * Length of the contents stored by current `YInput` cell.
   *
   * For [Y_JSON_NULL] and [Y_JSON_UNDEF] its equal to `0`.
   *
   * For [Y_JSON_ARR], [Y_JSON_MAP], [Y_ARRAY] and [Y_MAP] it describes a number of passed
   * elements.
   *
   * For other types it's always equal to `1`.
   */
  int len;
  /**
   * Union struct which contains a content corresponding to a provided `tag` field.
   */
  union YInputContent value;
} YInput;

/**
 * Iterator structure used by shared array data type.
 */
typedef YArrayIter YArrayIter;

/**
 * Iterator structure used by shared map data type. Map iterators are unordered - there's no
 * specific order in which map entries will be returned during consecutive iterator calls.
 */
typedef YMapIter YMapIter;

/**
 * Iterator structure used by XML nodes (elements and text) to iterate over node's attributes.
 * Attribute iterators are unordered - there's no specific order in which map entries will be
 * returned during consecutive iterator calls.
 */
typedef YXmlAttrIter YXmlAttrIter;

/**
 * Iterator used to traverse over the complex nested tree structure of a XML node. XML node
 * iterator walks only over `YXmlElement` and `YXmlText` nodes. It does so in ordered manner (using
 * the order in which children are ordered within their parent nodes) and using **depth-first**
 * traverse.
 */
typedef YXmlTreeWalker YXmlTreeWalker;

/**
 * Event pushed into callbacks registered with `ytext_observe` function. It contains delta of all
 * text changes made within a scope of corresponding transaction (see: `ytext_event_delta`) as
 * well as navigation data used to identify a `YText` instance which triggered this event.
 */
typedef struct YTextEvent {
  const void *inner;
  const YTransaction *txn;
} YTextEvent;

/**
 * Event pushed into callbacks registered with `ymap_observe` function. It contains all
 * key-value changes made within a scope of corresponding transaction (see: `ymap_event_keys`) as
 * well as navigation data used to identify a `YMap` instance which triggered this event.
 */
typedef struct YMapEvent {
  const void *inner;
  const YTransaction *txn;
} YMapEvent;

/**
 * Event pushed into callbacks registered with `yarray_observe` function. It contains delta of all
 * content changes made within a scope of corresponding transaction (see: `yarray_event_delta`) as
 * well as navigation data used to identify a `YArray` instance which triggered this event.
 */
typedef struct YArrayEvent {
  const void *inner;
  const YTransaction *txn;
} YArrayEvent;

/**
 * Event pushed into callbacks registered with `yxmlelem_observe` function. It contains
 * all attribute changes made within a scope of corresponding transaction
 * (see: `yxmlelem_event_keys`) as well as child XML nodes changes (see: `yxmlelem_event_delta`)
 * and navigation data used to identify a `YXmlElement` instance which triggered this event.
 */
typedef struct YXmlEvent {
  const void *inner;
  const YTransaction *txn;
} YXmlEvent;

/**
 * Event pushed into callbacks registered with `yxmltext_observe` function. It contains
 * all attribute changes made within a scope of corresponding transaction
 * (see: `yxmltext_event_keys`) as well as text edits (see: `yxmltext_event_delta`)
 * and navigation data used to identify a `YXmlText` instance which triggered this event.
 */
typedef struct YXmlTextEvent {
  const void *inner;
  const YTransaction *txn;
} YXmlTextEvent;

typedef union YEventContent {
  struct YTextEvent text;
  struct YMapEvent map;
  struct YArrayEvent array;
  struct YXmlEvent xml_elem;
  struct YXmlTextEvent xml_text;
} YEventContent;

typedef struct YEvent {
  /**
   * Tag describing, which shared type emitted this event.
   *
   * - [Y_TEXT] for pointers to `YText` data types.
   * - [Y_ARRAY] for pointers to `YArray` data types.
   * - [Y_MAP] for pointers to `YMap` data types.
   * - [Y_XML_ELEM] for pointers to `YXmlElement` data types.
   * - [Y_XML_TEXT] for pointers to `YXmlText` data types.
   */
  int8_t tag;
  /**
   * A nested event type, specific for a shared data type that triggered it. Type of an
   * event can be verified using `tag` field.
   */
  union YEventContent content;
} YEvent;

typedef union YPathSegmentCase {
  const char *key;
  int index;
} YPathSegmentCase;

/**
 * A single segment of a path returned from `yevent_path` function. It can be one of two cases,
 * recognized by it's `tag` field:
 *
 * 1. `Y_EVENT_PATH_KEY` means that segment value can be accessed by `segment.value.key` and is
 * referring to a string key used by map component (eg. `YMap` entry).
 * 2. `Y_EVENT_PATH_INDEX` means that segment value can be accessed by `segment.value.index` and is
 * referring to an int index used by sequence component (eg. `YArray` item or `YXmlElement` child).
 */
typedef struct YPathSegment {
  /**
   * Tag used to identify which case current segment is referring to:
   *
   * 1. `Y_EVENT_PATH_KEY` means that segment value can be accessed by `segment.value.key` and is
   * referring to a string key used by map component (eg. `YMap` entry).
   * 2. `Y_EVENT_PATH_INDEX` means that segment value can be accessed by `segment.value.index`
   * and is referring to an int index used by sequence component (eg. `YArray` item or
   * `YXmlElement` child).
   */
  char tag;
  /**
   * Union field containing either `key` or `index`. A particular case can be recognized by using
   * segment's `tag` field.
   */
  union YPathSegmentCase value;
} YPathSegment;

/**
 * A single instance of formatting attribute stored as part of `YDelta` instance.
 */
typedef struct YDeltaAttr {
  /**
   * A null-terminated UTF-8 encoded string containing a unique formatting attribute name.
   */
  const char *key;
  /**
   * A value assigned to a formatting attribute.
   */
  struct YOutput value;
} YDeltaAttr;

/**
 * A data type representing a single change detected over an observed `YText`/`YXmlText`. A type
 * of change can be detected using a `tag` field:
 *
 * 1. `Y_EVENT_CHANGE_ADD` marks a new characters added to a collection. In this case `insert`
 * field contains a pointer to a list of newly inserted values, while `len` field informs about
 * their count. Additionally `attributes_len` nad `attributes` carry information about optional
 * formatting attributes applied to edited blocks.
 * 2. `Y_EVENT_CHANGE_DELETE` marks an existing elements removed from the collection. In this case
 * `len` field informs about number of removed elements.
 * 3. `Y_EVENT_CHANGE_RETAIN` marks a number of characters that have not been changed, counted from
 * the previous element. `len` field informs about number of retained elements. Additionally
 * `attributes_len` nad `attributes` carry information about optional formatting attributes applied
 * to edited blocks.
 *
 * A list of changes returned by `ytext_event_delta`/`yxmltext_event_delta` enables to locate
 * a position of all changes within an observed collection by using a combination of added/deleted
 * change structs separated by retained changes (marking eg. number of elements that can be safely
 * skipped, since they remained unchanged).
 */
typedef struct YDelta {
  /**
   * Tag field used to identify particular type of change made:
   *
   * 1. `Y_EVENT_CHANGE_ADD` marks a new elements added to a collection. In this case `values`
   * field contains a pointer to a list of newly inserted values, while `len` field informs about
   * their count.
   * 2. `Y_EVENT_CHANGE_DELETE` marks an existing elements removed from the collection. In this
   * case `len` field informs about number of removed elements.
   * 3. `Y_EVENT_CHANGE_RETAIN` marks a number of elements that have not been changed, counted
   * from the previous element. `len` field informs about number of retained elements.
   */
  char tag;
  /**
   * Number of element affected by current type of a change. It can refer to a number of
   * inserted `values`, number of deleted element or a number of retained (unchanged) values.
   */
  int len;
  /**
   * Used in case when current change is of `Y_EVENT_CHANGE_ADD` type. Contains a list (of
   * length stored in `len` field) of newly inserted values.
   */
  struct YOutput *insert;
  /**
   * A number of formatting attributes assigned to an edited area represented by this delta.
   */
  int attributes_len;
  /**
   * A nullable pointer to a list of formatting attributes assigned to an edited area represented
   * by this delta.
   */
  struct YDeltaAttr *attributes;
} YDelta;

/**
 * A data type representing a single change detected over an observed shared collection. A type
 * of change can be detected using a `tag` field:
 *
 * 1. `Y_EVENT_CHANGE_ADD` marks a new elements added to a collection. In this case `values` field
 * contains a pointer to a list of newly inserted values, while `len` field informs about their
 * count.
 * 2. `Y_EVENT_CHANGE_DELETE` marks an existing elements removed from the collection. In this case
 * `len` field informs about number of removed elements.
 * 3. `Y_EVENT_CHANGE_RETAIN` marks a number of elements that have not been changed, counted from
 * the previous element. `len` field informs about number of retained elements.
 *
 * A list of changes returned by `yarray_event_delta`/`yxml_event_delta` enables to locate a
 * position of all changes within an observed collection by using a combination of added/deleted
 * change structs separated by retained changes (marking eg. number of elements that can be safely
 * skipped, since they remained unchanged).
 */
typedef struct YEventChange {
  /**
   * Tag field used to identify particular type of change made:
   *
   * 1. `Y_EVENT_CHANGE_ADD` marks a new elements added to a collection. In this case `values`
   * field contains a pointer to a list of newly inserted values, while `len` field informs about
   * their count.
   * 2. `Y_EVENT_CHANGE_DELETE` marks an existing elements removed from the collection. In this
   * case `len` field informs about number of removed elements.
   * 3. `Y_EVENT_CHANGE_RETAIN` marks a number of elements that have not been changed, counted
   * from the previous element. `len` field informs about number of retained elements.
   */
  char tag;
  /**
   * Number of element affected by current type of a change. It can refer to a number of
   * inserted `values`, number of deleted element or a number of retained (unchanged) values.
   */
  int len;
  /**
   * Used in case when current change is of `Y_EVENT_CHANGE_ADD` type. Contains a list (of
   * length stored in `len` field) of newly inserted values.
   */
  const struct YOutput *values;
} YEventChange;

/**
 * A data type representing a single change made over a map component of shared collection types,
 * such as `YMap` entries or `YXmlText`/`YXmlElement` attributes. A `key` field provides a
 * corresponding unique key string of a changed entry, while `tag` field informs about specific
 * type of change being done:
 *
 * 1. `Y_EVENT_KEY_CHANGE_ADD` used to identify a newly added entry. In this case an `old_value`
 * field is NULL, while `new_value` field contains an inserted value.
 * 1. `Y_EVENT_KEY_CHANGE_DELETE` used to identify an existing entry being removed. In this case
 * an `old_value` field contains the removed value.
 * 1. `Y_EVENT_KEY_CHANGE_UPDATE` used to identify an existing entry, which value has been changed.
 * In this case `old_value` field contains replaced value, while `new_value` contains a newly
 * inserted one.
 */
typedef struct YEventKeyChange {
  /**
   * A UTF8-encoded null-terminated string containing a key of a changed entry.
   */
  const char *key;
  /**
   * Tag field informing about type of change current struct refers to:
   *
   * 1. `Y_EVENT_KEY_CHANGE_ADD` used to identify a newly added entry. In this case an
   * `old_value` field is NULL, while `new_value` field contains an inserted value.
   * 1. `Y_EVENT_KEY_CHANGE_DELETE` used to identify an existing entry being removed. In this
   * case an `old_value` field contains the removed value.
   * 1. `Y_EVENT_KEY_CHANGE_UPDATE` used to identify an existing entry, which value has been
   * changed. In this case `old_value` field contains replaced value, while `new_value` contains
   * a newly inserted one.
   */
  char tag;
  /**
   * Contains a removed entry's value or replaced value of an updated entry.
   */
  const struct YOutput *old_value;
  /**
   * Contains a value of newly inserted entry or an updated entry's new value.
   */
  const struct YOutput *new_value;
} YEventKeyChange;

/**
 * Releases all memory-allocated resources bound to given document.
 */
void ydoc_destroy(YDoc *value);

/**
 * Frees all memory-allocated resources bound to a given [YMapEntry].
 */
void ymap_entry_destroy(struct YMapEntry *value);

/**
 * Frees all memory-allocated resources bound to a given [YXmlAttr].
 */
void yxmlattr_destroy(struct YXmlAttr *attr);

/**
 * Frees all memory-allocated resources bound to a given UTF-8 null-terminated string returned from
 * Yrs document API. Yrs strings don't use libc malloc, so calling `free()` on them will fault.
 */
void ystring_destroy(char *str);

/**
 * Frees all memory-allocated resources bound to a given binary returned from Yrs document API.
 * Unlike strings binaries are not null-terminated and can contain null characters inside,
 * therefore a size of memory to be released must be explicitly provided.
 * Yrs binaries don't use libc malloc, so calling `free()` on them will fault.
 */
void ybinary_destroy(unsigned char *ptr, int len);

/**
 * Creates a new [Doc] instance with a randomized unique client identifier.
 *
 * Use [ydoc_destroy] in order to release created [Doc] resources.
 */
YDoc *ydoc_new(void);

/**
 * Creates a new [Doc] instance with a specified `options`.
 *
 * Use [ydoc_destroy] in order to release created [Doc] resources.
 */
YDoc *ydoc_new_with_options(struct YOptions options);

/**
 * Returns a unique client identifier of this [Doc] instance.
 */
unsigned long ydoc_id(YDoc *doc);

unsigned int ydoc_observe_updates_v1(YDoc *doc,
                                     void *state,
                                     void (*cb)(void*, int, const unsigned char*));

unsigned int ydoc_observe_updates_v2(YDoc *doc,
                                     void *state,
                                     void (*cb)(void*, int, const unsigned char*));

void ydoc_unobserve_updates_v1(YDoc *doc, unsigned int subscription_id);

void ydoc_unobserve_updates_v2(YDoc *doc, unsigned int subscription_id);

unsigned int ydoc_observe_after_transaction(YDoc *doc,
                                            void *state,
                                            void (*cb)(void*, struct YAfterTransactionEvent*));

void ydoc_unobserve_after_transaction(YDoc *doc, unsigned int subscription_id);

/**
 * Starts a new read-write transaction on a given document. All other operations happen in context
 * of a transaction. Yrs transactions do not follow ACID rules. Once a set of operations is
 * complete, a transaction can be finished using [ytransaction_commit] function.
 */
YTransaction *ytransaction_new(YDoc *doc);

/**
 * Commit and dispose provided transaction. This operation releases allocated resources, triggers
 * update events and performs a storage compression over all operations executed in scope of
 * current transaction.
 */
void ytransaction_commit(YTransaction *txn);

/**
 * Gets or creates a new shared `YText` data type instance as a root-level type of a given document.
 * This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
 * compatible string.
 *
 * Use [ytext_destroy] in order to release pointer returned that way - keep in mind that this will
 * not remove `YText` instance from the document itself (once created it'll last for the entire
 * lifecycle of a document).
 */
Branch *ytext(YTransaction *txn, const char *name);

/**
 * Gets or creates a new shared `YArray` data type instance as a root-level type of a given document.
 * This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
 * compatible string.
 *
 * Use [yarray_destroy] in order to release pointer returned that way - keep in mind that this will
 * not remove `YArray` instance from the document itself (once created it'll last for the entire
 * lifecycle of a document).
 */
Branch *yarray(YTransaction *txn,
               const char *name);

/**
 * Gets or creates a new shared `YMap` data type instance as a root-level type of a given document.
 * This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
 * compatible string.
 *
 * Use [ymap_destroy] in order to release pointer returned that way - keep in mind that this will
 * not remove `YMap` instance from the document itself (once created it'll last for the entire
 * lifecycle of a document).
 */
Branch *ymap(YTransaction *txn, const char *name);

/**
 * Gets or creates a new shared `YXmlElement` data type instance as a root-level type of a given
 * document. This structure can later be accessed using its `name`, which must be a null-terminated
 * UTF-8 compatible string.
 *
 * Use [yxmlelem_destroy] in order to release pointer returned that way - keep in mind that this
 * will not remove `YXmlElement` instance from the document itself (once created it'll last for
 * the entire lifecycle of a document).
 */
Branch *yxmlelem(YTransaction *txn, const char *name);

/**
 * Gets or creates a new shared `YXmlText` data type instance as a root-level type of a given
 * document. This structure can later be accessed using its `name`, which must be a null-terminated
 * UTF-8 compatible string.
 *
 * Use [yxmltext_destroy] in order to release pointer returned that way - keep in mind that this
 * will not remove `YXmlText` instance from the document itself (once created it'll last for
 * the entire lifecycle of a document).
 */
Branch *yxmltext(YTransaction *txn, const char *name);

/**
 * Returns a state vector of a current transaction's document, serialized using lib0 version 1
 * encoding. Payload created by this function can then be send over the network to a remote peer,
 * where it can be used as a parameter of [ytransaction_state_diff_v1] in order to produce a delta
 * update payload, that can be send back and applied locally in order to efficiently propagate
 * updates from one peer to another.
 *
 * The length of a generated binary will be passed within a `len` out parameter.
 *
 * Once no longer needed, a returned binary can be disposed using [ybinary_destroy] function.
 */
unsigned char *ytransaction_state_vector_v1(const YTransaction *txn, int *len);

/**
 * Returns a delta difference between current state of a transaction's document and a state vector
 * `sv` encoded as a binary payload using lib0 version 1 encoding (which could be generated using
 * [ytransaction_state_vector_v1]). Such delta can be send back to the state vector's sender in
 * order to propagate and apply (using [ytransaction_apply]) all updates known to a current
 * document, which remote peer was not aware of.
 *
 * If passed `sv` pointer is null, the generated diff will be a snapshot containing entire state of
 * the document.
 *
 * A length of an encoded state vector payload must be passed as `sv_len` parameter.
 *
 * A length of generated delta diff binary will be passed within a `len` out parameter.
 *
 * Once no longer needed, a returned binary can be disposed using [ybinary_destroy] function.
 */
unsigned char *ytransaction_state_diff_v1(const YTransaction *txn,
                                          const unsigned char *sv,
                                          int sv_len,
                                          int *len);

/**
 * Returns a delta difference between current state of a transaction's document and a state vector
 * `sv` encoded as a binary payload using lib0 version 1 encoding (which could be generated using
 * [ytransaction_state_vector_v1]). Such delta can be send back to the state vector's sender in
 * order to propagate and apply (using [ytransaction_apply_v2]) all updates known to a current
 * document, which remote peer was not aware of.
 *
 * If passed `sv` pointer is null, the generated diff will be a snapshot containing entire state of
 * the document.
 *
 * A length of an encoded state vector payload must be passed as `sv_len` parameter.
 *
 * A length of generated delta diff binary will be passed within a `len` out parameter.
 *
 * Once no longer needed, a returned binary can be disposed using [ybinary_destroy] function.
 */
unsigned char *ytransaction_state_diff_v2(const YTransaction *txn,
                                          const unsigned char *sv,
                                          int sv_len,
                                          int *len);

/**
 * Returns a snapshot descriptor of a current state of the document. This snapshot information
 * can be then used to encode document data at a particular point in time
 * (see: `ytransaction_encode_state_from_snapshot`).
 */
unsigned char *ytransaction_snapshot(const YTransaction *txn, int *len);

/**
 * Encodes a state of the document at a point in time specified by the provided `snapshot`
 * (generated by: `ytransaction_snapshot`). This is useful to generate a past view of the document.
 *
 * The returned update is binary compatible with Yrs update lib0 v1 encoding, and can be processed
 * with functions dedicated to work on it, like `ytransaction_apply`.
 *
 * This function requires document with a GC option flag turned off (otherwise "time travel" would
 * not be a safe operation). If this is not a case, the NULL pointer will be returned.
 */
unsigned char *ytransaction_encode_state_from_snapshot_v1(const YTransaction *txn,
                                                          const unsigned char *snapshot,
                                                          int snapshot_len,
                                                          int *len);

/**
 * Encodes a state of the document at a point in time specified by the provided `snapshot`
 * (generated by: `ytransaction_snapshot`). This is useful to generate a past view of the document.
 *
 * The returned update is binary compatible with Yrs update lib0 v2 encoding, and can be processed
 * with functions dedicated to work on it, like `ytransaction_apply_v2`.
 *
 * This function requires document with a GC option flag turned off (otherwise "time travel" would
 * not be a safe operation). If this is not a case, the NULL pointer will be returned.
 */
unsigned char *ytransaction_encode_state_from_snapshot_v2(const YTransaction *txn,
                                                          const unsigned char *snapshot,
                                                          int snapshot_len,
                                                          int *len);

/**
 * Returns a null-terminated UTF-8 encoded string representation of an `update` binary payload,
 * encoded using lib0 v1 encoding.
 * Returns null if update couldn't be parsed into a lib0 v1 formatting.
 */
char *yupdate_debug_v1(const unsigned char *update, int update_len);

/**
 * Returns a null-terminated UTF-8 encoded string representation of an `update` binary payload,
 * encoded using lib0 v2 encoding.
 * Returns null if update couldn't be parsed into a lib0 v2 formatting.
 */
char *yupdate_debug_v2(const unsigned char *update, int update_len);

/**
 * Applies an diff update (generated by `ytransaction_state_diff_v1`) to a local transaction's
 * document.
 *
 * A length of generated `diff` binary must be passed within a `diff_len` out parameter.
 *
 * Returns an error code in case if transaction succeeded failed:
 * - **0**: success
 * - `ERR_CODE_IO` (**1**): couldn't read data from input stream.
 * - `ERR_CODE_VAR_INT` (**2**): decoded variable integer outside of the expected integer size bounds.
 * - `ERR_CODE_EOS` (**3**): end of stream found when more data was expected.
 * - `ERR_CODE_UNEXPECTED_VALUE` (**4**): decoded enum tag value was not among known cases.
 * - `ERR_CODE_INVALID_JSON` (**5**): failure when trying to decode JSON content.
 * - `ERR_CODE_OTHER` (**6**): other error type than the one specified.
 */
int ytransaction_apply(YTransaction *txn,
                       const unsigned char *diff,
                       int diff_len);

/**
 * Applies an diff update (generated by [ytransaction_state_diff_v2]) to a local transaction's
 * document.
 *
 * A length of generated `diff` binary must be passed within a `diff_len` out parameter.
 *
 * Returns an error code in case if transaction succeeded failed:
 * - **0**: success
 * - `ERR_CODE_IO` (**1**): couldn't read data from input stream.
 * - `ERR_CODE_VAR_INT` (**2**): decoded variable integer outside of the expected integer size bounds.
 * - `ERR_CODE_EOS` (**3**): end of stream found when more data was expected.
 * - `ERR_CODE_UNEXPECTED_VALUE` (**4**): decoded enum tag value was not among known cases.
 * - `ERR_CODE_INVALID_JSON` (**5**): failure when trying to decode JSON content.
 * - `ERR_CODE_OTHER` (**6**): other error type than the one specified.
 */
int ytransaction_apply_v2(YTransaction *txn,
                          const unsigned char *diff,
                          int diff_len);

/**
 * Returns the length of the `YText` string content in bytes (without the null terminator character)
 */
int ytext_len(const Branch *txt);

/**
 * Returns a null-terminated UTF-8 encoded string content of a current `YText` shared data type.
 *
 * Generated string resources should be released using [ystring_destroy] function.
 */
char *ytext_string(const Branch *txt);

/**
 * Inserts a null-terminated UTF-8 encoded string a given `index`. `index` value must be between
 * 0 and a length of a `YText` (inclusive, accordingly to [ytext_len] return value), otherwise this
 * function will panic.
 *
 * A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
 * ownership over a passed value - it will be copied and therefore a string parameter must be
 * released by the caller.
 *
 * A nullable pointer with defined `attrs` will be used to wrap provided text with
 * a formatting blocks. `attrs` must be a map-like type.
 */
void ytext_insert(const Branch *txt,
                  YTransaction *txn,
                  int index,
                  const char *value,
                  const struct YInput *attrs);

/**
 * Wraps an existing piece of text within a range described by `index`-`len` parameters with
 * formatting blocks containing provided `attrs` metadata. `attrs` must be a map-like type.
 */
void ytext_format(const Branch *txt,
                  YTransaction *txn,
                  int index,
                  int len,
                  const struct YInput *attrs);

/**
 * Inserts an embed content given `index`. `index` value must be between 0 and a length of a
 * `YText` (inclusive, accordingly to [ytext_len] return value), otherwise this
 * function will panic.
 *
 * A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
 * ownership over a passed value - it will be copied and therefore a string parameter must be
 * released by the caller.
 *
 * A nullable pointer with defined `attrs` will be used to wrap provided text with
 * a formatting blocks. `attrs` must be a map-like type.
 */
void ytext_insert_embed(const Branch *txt,
                        YTransaction *txn,
                        int index,
                        const struct YInput *content,
                        const struct YInput *attrs);

/**
 * Removes a range of characters, starting a a given `index`. This range must fit within the bounds
 * of a current `YText`, otherwise this function call will fail.
 *
 * An `index` value must be between 0 and the length of a `YText` (exclusive, accordingly to
 * [ytext_len] return value).
 *
 * A `length` must be lower or equal number of characters (counted as UTF chars depending on the
 * encoding configured by `YDoc`) from `index` position to the end of of the string.
 */
void ytext_remove_range(const Branch *txt, YTransaction *txn, int index, int length);

/**
 * Returns a number of elements stored within current instance of `YArray`.
 */
int yarray_len(const Branch *array);

/**
 * Returns a pointer to a `YOutput` value stored at a given `index` of a current `YArray`.
 * If `index` is outside of the bounds of an array, a null pointer will be returned.
 *
 * A value returned should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yarray_get(const Branch *array, int index);

/**
 * Inserts a range of `items` into current `YArray`, starting at given `index`. An `items_len`
 * parameter is used to determine the size of `items` array - it can also be used to insert
 * a single element given its pointer.
 *
 * An `index` value must be between 0 and (inclusive) length of a current array (use [yarray_len]
 * to determine its length), otherwise it will panic at runtime.
 *
 * `YArray` doesn't take ownership over the inserted `items` data - their contents are being copied
 * into array structure - therefore caller is responsible for freeing all memory associated with
 * input params.
 */
void yarray_insert_range(const Branch *array,
                         YTransaction *txn,
                         int index,
                         const struct YInput *items,
                         int items_len);

/**
 * Removes a `len` of consecutive range of elements from current `array` instance, starting at
 * a given `index`. Range determined by `index` and `len` must fit into boundaries of an array,
 * otherwise it will panic at runtime.
 */
void yarray_remove_range(const Branch *array, YTransaction *txn, int index, int len);

void yarray_move(const Branch *array, YTransaction *txn, int source, int target);

/**
 * Returns an iterator, which can be used to traverse over all elements of an `array` (`array`'s
 * length can be determined using [yarray_len] function).
 *
 * Use [yarray_iter_next] function in order to retrieve a consecutive array elements.
 * Use [yarray_iter_destroy] function in order to close the iterator and release its resources.
 */
YArrayIter *yarray_iter(const Branch *array);

/**
 * Releases all of an `YArray` iterator resources created by calling [yarray_iter].
 */
void yarray_iter_destroy(YArrayIter *iter);

/**
 * Moves current `YArray` iterator over to a next element, returning a pointer to it. If an iterator
 * comes to an end of an array, a null pointer will be returned.
 *
 * Returned values should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yarray_iter_next(YArrayIter *iterator);

/**
 * Returns an iterator, which can be used to traverse over all key-value pairs of a `map`.
 *
 * Use [ymap_iter_next] function in order to retrieve a consecutive (**unordered**) map entries.
 * Use [ymap_iter_destroy] function in order to close the iterator and release its resources.
 */
YMapIter *ymap_iter(const Branch *map);

/**
 * Releases all of an `YMap` iterator resources created by calling [ymap_iter].
 */
void ymap_iter_destroy(YMapIter *iter);

/**
 * Moves current `YMap` iterator over to a next entry, returning a pointer to it. If an iterator
 * comes to an end of a map, a null pointer will be returned. Yrs maps are unordered and so are
 * their iterators.
 *
 * Returned values should be eventually released using [ymap_entry_destroy] function.
 */
struct YMapEntry *ymap_iter_next(YMapIter *iter);

/**
 * Returns a number of entries stored within a `map`.
 */
int ymap_len(const Branch *map);

/**
 * Inserts a new entry (specified as `key`-`value` pair) into a current `map`. If entry under such
 * given `key` already existed, its corresponding value will be replaced.
 *
 * A `key` must be a null-terminated UTF-8 encoded string, which contents will be copied into
 * a `map` (therefore it must be freed by the function caller).
 *
 * A `value` content is being copied into a `map`, therefore any of its content must be freed by
 * the function caller.
 */
void ymap_insert(const Branch *map, YTransaction *txn, const char *key, const struct YInput *value);

/**
 * Removes a `map` entry, given its `key`. Returns `1` if the corresponding entry was successfully
 * removed or `0` if no entry with a provided `key` has been found inside of a `map`.
 *
 * A `key` must be a null-terminated UTF-8 encoded string.
 */
char ymap_remove(const Branch *map, YTransaction *txn, const char *key);

/**
 * Returns a value stored under the provided `key`, or a null pointer if no entry with such `key`
 * has been found in a current `map`. A returned value is allocated by this function and therefore
 * should be eventually released using [youtput_destroy] function.
 *
 * A `key` must be a null-terminated UTF-8 encoded string.
 */
struct YOutput *ymap_get(const Branch *map, const char *key);

/**
 * Removes all entries from a current `map`.
 */
void ymap_remove_all(const Branch *map, YTransaction *txn);

/**
 * Return a name (or an XML tag) of a current `YXmlElement`. Root-level XML nodes use "UNDEFINED" as
 * their tag names.
 *
 * Returned value is a null-terminated UTF-8 string, which must be released using [ystring_destroy]
 * function.
 */
char *yxmlelem_tag(const Branch *xml);

/**
 * Converts current `YXmlElement` together with its children and attributes into a flat string
 * representation (no padding) eg. `<UNDEFINED><title key="value">sample text</title></UNDEFINED>`.
 *
 * Returned value is a null-terminated UTF-8 string, which must be released using [ystring_destroy]
 * function.
 */
char *yxmlelem_string(const Branch *xml);

/**
 * Inserts an XML attribute described using `attr_name` and `attr_value`. If another attribute with
 * the same name already existed, its value will be replaced with a provided one.
 *
 * Both `attr_name` and `attr_value` must be a null-terminated UTF-8 encoded strings. Their
 * contents are being copied, therefore it's up to a function caller to properly release them.
 */
void yxmlelem_insert_attr(const Branch *xml,
                          YTransaction *txn,
                          const char *attr_name,
                          const char *attr_value);

/**
 * Removes an attribute from a current `YXmlElement`, given its name.
 *
 * An `attr_name`must be a null-terminated UTF-8 encoded string.
 */
void yxmlelem_remove_attr(const Branch *xml, YTransaction *txn, const char *attr_name);

/**
 * Returns the value of a current `YXmlElement`, given its name, or a null pointer if not attribute
 * with such name has been found. Returned pointer is a null-terminated UTF-8 encoded string, which
 * should be released using [ystring_destroy] function.
 *
 * An `attr_name` must be a null-terminated UTF-8 encoded string.
 */
char *yxmlelem_get_attr(const Branch *xml, const char *attr_name);

/**
 * Returns an iterator over the `YXmlElement` attributes.
 *
 * Use [yxmlattr_iter_next] function in order to retrieve a consecutive (**unordered**) attributes.
 * Use [yxmlattr_iter_destroy] function in order to close the iterator and release its resources.
 */
YXmlAttrIter *yxmlelem_attr_iter(const Branch *xml);

/**
 * Returns an iterator over the `YXmlText` attributes.
 *
 * Use [yxmlattr_iter_next] function in order to retrieve a consecutive (**unordered**) attributes.
 * Use [yxmlattr_iter_destroy] function in order to close the iterator and release its resources.
 */
YXmlAttrIter *yxmltext_attr_iter(const Branch *xml);

/**
 * Releases all of attributes iterator resources created by calling [yxmlelem_attr_iter]
 * or [yxmltext_attr_iter].
 */
void yxmlattr_iter_destroy(YXmlAttrIter *iterator);

/**
 * Returns a next XML attribute from an `iterator`. Attributes are returned in an unordered
 * manner. Once `iterator` reaches the end of attributes collection, a null pointer will be
 * returned.
 *
 * Returned value should be eventually released using [yxmlattr_destroy].
 */
struct YXmlAttr *yxmlattr_iter_next(YXmlAttrIter *iterator);

/**
 * Returns a next sibling of a current `YXmlElement`, which can be either another `YXmlElement`
 * or a `YXmlText`. Together with [yxmlelem_first_child] it may be used to iterate over the direct
 * children of an XML node (in order to iterate over the nested XML structure use
 * [yxmlelem_tree_walker]).
 *
 * If current `YXmlElement` is the last child, this function returns a null pointer.
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmlelem_next_sibling(const Branch *xml);

/**
 * Returns a previous sibling of a current `YXmlElement`, which can be either another `YXmlElement`
 * or a `YXmlText`.
 *
 * If current `YXmlElement` is the first child, this function returns a null pointer.
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmlelem_prev_sibling(const Branch *xml);

/**
 * Returns a next sibling of a current `YXmlText`, which can be either another `YXmlText` or
 * an `YXmlElement`. Together with [yxmlelem_first_child] it may be used to iterate over the direct
 * children of an XML node (in order to iterate over the nested XML structure use
 * [yxmlelem_tree_walker]).
 *
 * If current `YXmlText` is the last child, this function returns a null pointer.
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmltext_next_sibling(const Branch *xml);

/**
 * Returns a previous sibling of a current `YXmlText`, which can be either another `YXmlText` or
 * an `YXmlElement`.
 *
 * If current `YXmlText` is the first child, this function returns a null pointer.
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmltext_prev_sibling(const Branch *xml);

/**
 * Returns a parent `YXmlElement` of a current node, or null pointer when current `YXmlElement` is
 * a root-level shared data type.
 *
 * A returned value should be eventually released using [youtput_destroy] function.
 */
Branch *yxmlelem_parent(const Branch *xml);

/**
 * Returns a number of child nodes (both `YXmlElement` and `YXmlText`) living under a current XML
 * element. This function doesn't count a recursive nodes, only direct children of a current node.
 */
int yxmlelem_child_len(const Branch *xml);

/**
 * Returns a first child node of a current `YXmlElement`, or null pointer if current XML node is
 * empty. Returned value could be either another `YXmlElement` or `YXmlText`.
 *
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmlelem_first_child(const Branch *xml);

/**
 * Returns an iterator over a nested recursive structure of a current `YXmlElement`, starting from
 * first of its children. Returned values can be either `YXmlElement` or `YXmlText` nodes.
 *
 * Use [yxmlelem_tree_walker_next] function in order to iterate over to a next node.
 * Use [yxmlelem_tree_walker_destroy] function to release resources used by the iterator.
 */
YXmlTreeWalker *yxmlelem_tree_walker(const Branch *xml);

/**
 * Releases resources associated with a current XML tree walker iterator.
 */
void yxmlelem_tree_walker_destroy(YXmlTreeWalker *iter);

/**
 * Moves current `iterator` to a next value (either `YXmlElement` or `YXmlText`), returning its
 * pointer or a null, if an `iterator` already reached the last successor node.
 *
 * Values returned by this function should be eventually released using [youtput_destroy].
 */
struct YOutput *yxmlelem_tree_walker_next(YXmlTreeWalker *iterator);

/**
 * Inserts an `YXmlElement` as a child of a current node at the given `index` and returns its
 * pointer. Node created this way will have a given `name` as its tag (eg. `p` for `<p></p>` node).
 *
 * An `index` value must be between 0 and (inclusive) length of a current XML element (use
 * [yxmlelem_child_len] function to determine its length).
 *
 * A `name` must be a null-terminated UTF-8 encoded string, which will be copied into current
 * document. Therefore `name` should be freed by the function caller.
 */
Branch *yxmlelem_insert_elem(const Branch *xml, YTransaction *txn, int index, const char *name);

/**
 * Inserts an `YXmlText` as a child of a current node at the given `index` and returns its
 * pointer.
 *
 * An `index` value must be between 0 and (inclusive) length of a current XML element (use
 * [yxmlelem_child_len] function to determine its length).
 */
Branch *yxmlelem_insert_text(const Branch *xml, YTransaction *txn, int index);

/**
 * Removes a consecutive range of child elements (of specified length) from the current
 * `YXmlElement`, starting at the given `index`. Specified range must fit into boundaries of current
 * XML node children, otherwise this function will panic at runtime.
 */
void yxmlelem_remove_range(const Branch *xml, YTransaction *txn, int index, int len);

/**
 * Returns an XML child node (either a `YXmlElement` or `YXmlText`) stored at a given `index` of
 * a current `YXmlElement`. Returns null pointer if `index` was outside of the bound of current XML
 * node children.
 *
 * Returned value should be eventually released using [youtput_destroy].
 */
const struct YOutput *yxmlelem_get(const Branch *xml, int index);

/**
 * Returns the length of the `YXmlText` string content in bytes (without the null terminator
 * character)
 */
int yxmltext_len(const Branch *txt);

/**
 * Returns a null-terminated UTF-8 encoded string content of a current `YXmlText` shared data type.
 *
 * Generated string resources should be released using [ystring_destroy] function.
 */
char *yxmltext_string(const Branch *txt);

/**
 * Inserts a null-terminated UTF-8 encoded string a a given `index`. `index` value must be between
 * 0 and a length of a `YXmlText` (inclusive, accordingly to [yxmltext_len] return value), otherwise
 * this function will panic.
 *
 * A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
 * ownership over a passed value - it will be copied and therefore a string parameter must be
 * released by the caller.
 *
 * A nullable pointer with defined `attrs` will be used to wrap provided text with
 * a formatting blocks. `attrs` must be a map-like type.
 */
void yxmltext_insert(const Branch *txt,
                     YTransaction *txn,
                     int index,
                     const char *str,
                     const struct YInput *attrs);

/**
 * Inserts an embed content given `index`. `index` value must be between 0 and a length of a
 * `YXmlText` (inclusive, accordingly to [ytext_len] return value), otherwise this
 * function will panic.
 *
 * A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
 * ownership over a passed value - it will be copied and therefore a string parameter must be
 * released by the caller.
 *
 * A nullable pointer with defined `attrs` will be used to wrap provided text with
 * a formatting blocks. `attrs` must be a map-like type.
 */
void yxmltext_insert_embed(const Branch *txt,
                           YTransaction *txn,
                           int index,
                           const struct YInput *content,
                           const struct YInput *attrs);

/**
 * Wraps an existing piece of text within a range described by `index`-`len` parameters with
 * formatting blocks containing provided `attrs` metadata. `attrs` must be a map-like type.
 */
void yxmltext_format(const Branch *txt,
                     YTransaction *txn,
                     int index,
                     int len,
                     const struct YInput *attrs);

/**
 * Removes a range of characters, starting a a given `index`. This range must fit within the bounds
 * of a current `YXmlText`, otherwise this function call will fail.
 *
 * An `index` value must be between 0 and the length of a `YXmlText` (exclusive, accordingly to
 * [yxmltext_len] return value).
 *
 * A `length` must be lower or equal number of characters (counted as UTF chars depending on the
 * encoding configured by `YDoc`) from `index` position to the end of of the string.
 */
void yxmltext_remove_range(const Branch *txt, YTransaction *txn, int idx, int len);

/**
 * Inserts an XML attribute described using `attr_name` and `attr_value`. If another attribute with
 * the same name already existed, its value will be replaced with a provided one.
 *
 * Both `attr_name` and `attr_value` must be a null-terminated UTF-8 encoded strings. Their
 * contents are being copied, therefore it's up to a function caller to properly release them.
 */
void yxmltext_insert_attr(const Branch *txt,
                          YTransaction *txn,
                          const char *attr_name,
                          const char *attr_value);

/**
 * Removes an attribute from a current `YXmlText`, given its name.
 *
 * An `attr_name`must be a null-terminated UTF-8 encoded string.
 */
void yxmltext_remove_attr(const Branch *txt, YTransaction *txn, const char *attr_name);

/**
 * Returns the value of a current `YXmlText`, given its name, or a null pointer if not attribute
 * with such name has been found. Returned pointer is a null-terminated UTF-8 encoded string, which
 * should be released using [ystring_destroy] function.
 *
 * An `attr_name` must be a null-terminated UTF-8 encoded string.
 */
char *yxmltext_get_attr(const Branch *txt, const char *attr_name);

/**
 * Releases all resources related to a corresponding `YOutput` cell.
 */
void youtput_destroy(struct YOutput *val);

/**
 * Function constructor used to create JSON-like NULL `YInput` cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_null(void);

/**
 * Function constructor used to create JSON-like undefined `YInput` cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_undefined(void);

/**
 * Function constructor used to create JSON-like boolean `YInput` cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_bool(char flag);

/**
 * Function constructor used to create JSON-like 64-bit floating point number `YInput` cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_float(float num);

/**
 * Function constructor used to create JSON-like 64-bit signed integer `YInput` cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_long(long integer);

/**
 * Function constructor used to create a string `YInput` cell. Provided parameter must be
 * a null-terminated UTF-8 encoded string. This function doesn't allocate any heap resources,
 * and doesn't release any on its own, therefore its up to a caller to free resources once
 * a structure is no longer needed.
 */
struct YInput yinput_string(const char *str);

/**
 * Function constructor used to create a binary `YInput` cell of a specified length.
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_binary(const uint8_t *buf, int len);

/**
 * Function constructor used to create a JSON-like array `YInput` cell of other JSON-like values of
 * a given length. This function doesn't allocate any heap resources and doesn't release any on its
 * own, therefore its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_json_array(struct YInput *values, int len);

/**
 * Function constructor used to create a JSON-like map `YInput` cell of other JSON-like key-value
 * pairs. These pairs are build from corresponding indexes of `keys` and `values`, which must have
 * the same specified length.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_json_map(char **keys, struct YInput *values, int len);

/**
 * Function constructor used to create a nested `YArray` `YInput` cell prefilled with other
 * values of a given length. This function doesn't allocate any heap resources and doesn't release
 * any on its own, therefore its up to a caller to free resources once a structure is no longer
 * needed.
 */
struct YInput yinput_yarray(struct YInput *values, int len);

/**
 * Function constructor used to create a nested `YMap` `YInput` cell prefilled with other key-value
 * pairs. These pairs are build from corresponding indexes of `keys` and `values`, which must have
 * the same specified length.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_ymap(char **keys, struct YInput *values, int len);

/**
 * Function constructor used to create a nested `YText` `YInput` cell prefilled with a specified
 * string, which must be a null-terminated UTF-8 character pointer.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_ytext(char *str);

/**
 * Function constructor used to create a nested `YXmlElement` `YInput` cell with a specified
 * tag name, which must be a null-terminated UTF-8 character pointer.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_yxmlelem(char *name);

/**
 * Function constructor used to create a nested `YXmlText` `YInput` cell prefilled with a specified
 * string, which must be a null-terminated UTF-8 character pointer.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_yxmltext(char *str);

/**
 * Attempts to read the value for a given `YOutput` pointer as a boolean flag, which can be either
 * `1` for truthy case and `0` otherwise. Returns a null pointer in case when a value stored under
 * current `YOutput` cell is not of a boolean type.
 */
const char *youtput_read_bool(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as a 64-bit floating point number.
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not a floating point number.
 */
const float *youtput_read_float(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as a 64-bit signed integer.
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not a signed integer.
 */
const long long *youtput_read_long(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as a null-terminated UTF-8 encoded
 * string.
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not a string. Underlying string is released automatically as part of [youtput_destroy]
 * destructor.
 */
char *youtput_read_string(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as a binary payload (which length is
 * stored within `len` filed of a cell itself).
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not a binary type. Underlying binary is released automatically as part of [youtput_destroy]
 * destructor.
 */
const unsigned char *youtput_read_binary(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as a JSON-like array of `YOutput`
 * values (which length is stored within `len` filed of a cell itself).
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not a JSON-like array. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
struct YOutput *youtput_read_json_array(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as a JSON-like map of key-value entries
 * (which length is stored within `len` filed of a cell itself).
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not a JSON-like map. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
struct YMapEntry *youtput_read_json_map(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as an `YArray`.
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not an `YArray`. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
Branch *youtput_read_yarray(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as an `YXmlElement`.
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not an `YXmlElement`. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
Branch *youtput_read_yxmlelem(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as an `YMap`.
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not an `YMap`. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
Branch *youtput_read_ymap(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as an `YText`.
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not an `YText`. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
Branch *youtput_read_ytext(const struct YOutput *val);

/**
 * Attempts to read the value for a given `YOutput` pointer as an `YXmlText`.
 *
 * Returns a null pointer in case when a value stored under current `YOutput` cell
 * is not an `YXmlText`. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
Branch *youtput_read_yxmltext(const struct YOutput *val);

/**
 * Subscribes a given callback function `cb` to changes made by this `YText` instance. Callbacks
 * are triggered whenever a `ytransaction_commit` is called.
 * Returns a subscription ID which can be then used to unsubscribe this callback by using
 * `ytext_unobserve` function.
 */
unsigned int ytext_observe(const Branch *txt,
                           void *state,
                           void (*cb)(void*, const struct YTextEvent*));

/**
 * Subscribes a given callback function `cb` to changes made by this `YMap` instance. Callbacks
 * are triggered whenever a `ytransaction_commit` is called.
 * Returns a subscription ID which can be then used to unsubscribe this callback by using
 * `ymap_unobserve` function.
 */
unsigned int ymap_observe(const Branch *map,
                          void *state,
                          void (*cb)(void*, const struct YMapEvent*));

/**
 * Subscribes a given callback function `cb` to changes made by this `YArray` instance. Callbacks
 * are triggered whenever a `ytransaction_commit` is called.
 * Returns a subscription ID which can be then used to unsubscribe this callback by using
 * `yarray_unobserve` function.
 */
unsigned int yarray_observe(const Branch *array,
                            void *state,
                            void (*cb)(void*, const struct YArrayEvent*));

/**
 * Subscribes a given callback function `cb` to changes made by this `YXmlElement` instance.
 * Callbacks are triggered whenever a `ytransaction_commit` is called.
 * Returns a subscription ID which can be then used to unsubscribe this callback by using
 * `yxmlelem_unobserve` function.
 */
unsigned int yxmlelem_observe(const Branch *xml,
                              void *state,
                              void (*cb)(void*, const struct YXmlEvent*));

/**
 * Subscribes a given callback function `cb` to changes made by this `YXmlText` instance. Callbacks
 * are triggered whenever a `ytransaction_commit` is called.
 * Returns a subscription ID which can be then used to unsubscribe this callback by using
 * `yxmltext_unobserve` function.
 */
unsigned int yxmltext_observe(const Branch *xml,
                              void *state,
                              void (*cb)(void*, const struct YXmlTextEvent*));

/**
 * Subscribes a given callback function `cb` to changes made by this shared type instance as well
 * as all nested shared types living within it. Callbacks are triggered whenever a
 * `ytransaction_commit` is called.
 *
 * Returns a subscription ID which can be then used to unsubscribe this callback by using
 * `yunobserve_deep` function.
 */
unsigned int yobserve_deep(Branch *ytype,
                           void *state,
                           void (*cb)(void*, int, const struct YEvent*));

/**
 * Releases a callback subscribed via `ytext_observe` function represented by passed
 * observer parameter.
 */
void ytext_unobserve(const Branch *txt, unsigned int subscription_id);

/**
 * Releases a callback subscribed via `yarray_observe` function represented by passed
 * observer parameter.
 */
void yarray_unobserve(const Branch *array, unsigned int subscription_id);

/**
 * Releases a callback subscribed via `ymap_observe` function represented by passed
 * observer parameter.
 */
void ymap_unobserve(const Branch *map, unsigned int subscription_id);

/**
 * Releases a callback subscribed via `yxmlelem_observe` function represented by passed
 * observer parameter.
 */
void yxmlelem_unobserve(const Branch *xml, unsigned int subscription_id);

/**
 * Releases a callback subscribed via `yxmltext_observe` function represented by passed
 * observer parameter.
 */
void yxmltext_unobserve(const Branch *xml, unsigned int subscription_id);

/**
 * Releases a callback subscribed via `yobserve_deep` function represented by passed
 * observer parameter.
 */
void yunobserve_deep(Branch *ytype, unsigned int subscription_id);

/**
 * Returns a pointer to a shared collection, which triggered passed event `e`.
 */
Branch *ytext_event_target(const struct YTextEvent *e);

/**
 * Returns a pointer to a shared collection, which triggered passed event `e`.
 */
Branch *yarray_event_target(const struct YArrayEvent *e);

/**
 * Returns a pointer to a shared collection, which triggered passed event `e`.
 */
Branch *ymap_event_target(const struct YMapEvent *e);

/**
 * Returns a pointer to a shared collection, which triggered passed event `e`.
 */
Branch *yxmlelem_event_target(const struct YXmlEvent *e);

/**
 * Returns a pointer to a shared collection, which triggered passed event `e`.
 */
Branch *yxmltext_event_target(const struct YXmlTextEvent *e);

/**
 * Returns a path from a root type down to a current shared collection (which can be obtained using
 * `ytext_event_target` function). It can consist of either integer indexes (used by sequence
 * components) of *char keys (used by map components). `len` output parameter is used to provide
 * information about length of the path.
 *
 * Path returned this way should be eventually released using `ypath_destroy`.
 */
struct YPathSegment *ytext_event_path(const struct YTextEvent *e, int *len);

/**
 * Returns a path from a root type down to a current shared collection (which can be obtained using
 * `ymap_event_target` function). It can consist of either integer indexes (used by sequence
 * components) of *char keys (used by map components). `len` output parameter is used to provide
 * information about length of the path.
 *
 * Path returned this way should be eventually released using `ypath_destroy`.
 */
struct YPathSegment *ymap_event_path(const struct YMapEvent *e, int *len);

/**
 * Returns a path from a root type down to a current shared collection (which can be obtained using
 * `yxmlelem_event_path` function). It can consist of either integer indexes (used by sequence
 * components) of *char keys (used by map components). `len` output parameter is used to provide
 * information about length of the path.
 *
 * Path returned this way should be eventually released using `ypath_destroy`.
 */
struct YPathSegment *yxmlelem_event_path(const struct YXmlEvent *e, int *len);

/**
 * Returns a path from a root type down to a current shared collection (which can be obtained using
 * `yxmltext_event_path` function). It can consist of either integer indexes (used by sequence
 * components) of *char keys (used by map components). `len` output parameter is used to provide
 * information about length of the path.
 *
 * Path returned this way should be eventually released using `ypath_destroy`.
 */
struct YPathSegment *yxmltext_event_path(const struct YXmlTextEvent *e, int *len);

/**
 * Returns a path from a root type down to a current shared collection (which can be obtained using
 * `yarray_event_target` function). It can consist of either integer indexes (used by sequence
 * components) of *char keys (used by map components). `len` output parameter is used to provide
 * information about length of the path.
 *
 * Path returned this way should be eventually released using `ypath_destroy`.
 */
struct YPathSegment *yarray_event_path(const struct YArrayEvent *e, int *len);

/**
 * Releases allocated memory used by objects returned from path accessor functions of shared type
 * events.
 */
void ypath_destroy(struct YPathSegment *path, int len);

/**
 * Returns a sequence of changes produced by sequence component of shared collections (such as
 * `YText`, `YXmlText` and XML nodes added to `YXmlElement`). `len` output parameter is used to
 * provide information about number of changes produced.
 *
 * Delta returned from this function should eventually be released using `yevent_delta_destroy`
 * function.
 */
struct YDelta *ytext_event_delta(const struct YTextEvent *e, int *len);

/**
 * Returns a sequence of changes produced by sequence component of shared collections (such as
 * `YText`, `YXmlText` and XML nodes added to `YXmlElement`). `len` output parameter is used to
 * provide information about number of changes produced.
 *
 * Delta returned from this function should eventually be released using `yevent_delta_destroy`
 * function.
 */
struct YDelta *yxmltext_event_delta(const struct YXmlTextEvent *e, int *len);

/**
 * Returns a sequence of changes produced by sequence component of shared collections (such as
 * `YText`, `YXmlText` and XML nodes added to `YXmlElement`). `len` output parameter is used to
 * provide information about number of changes produced.
 *
 * Delta returned from this function should eventually be released using `yevent_delta_destroy`
 * function.
 */
struct YEventChange *yarray_event_delta(const struct YArrayEvent *e, int *len);

/**
 * Returns a sequence of changes produced by sequence component of shared collections (such as
 * `YText`, `YXmlText` and XML nodes added to `YXmlElement`). `len` output parameter is used to
 * provide information about number of changes produced.
 *
 * Delta returned from this function should eventually be released using `yevent_delta_destroy`
 * function.
 */
struct YEventChange *yxmlelem_event_delta(const struct YXmlEvent *e, int *len);

/**
 * Releases memory allocated by the object returned from `yevent_delta` function.
 */
void ytext_delta_destroy(struct YDelta *delta, int len);

/**
 * Releases memory allocated by the object returned from `yevent_delta` function.
 */
void yevent_delta_destroy(struct YEventChange *delta, int len);

/**
 * Returns a sequence of changes produced by map component of shared collections (such as
 * `YMap` and `YXmlText`/`YXmlElement` attribute changes). `len` output parameter is used to
 * provide information about number of changes produced.
 *
 * Delta returned from this function should eventually be released using `yevent_keys_destroy`
 * function.
 */
struct YEventKeyChange *ymap_event_keys(const struct YMapEvent *e, int *len);

/**
 * Returns a sequence of changes produced by map component of shared collections.
 * `len` output parameter is used to provide information about number of changes produced.
 *
 * Delta returned from this function should eventually be released using `yevent_keys_destroy`
 * function.
 */
struct YEventKeyChange *yxmlelem_event_keys(const struct YXmlEvent *e, int *len);

/**
 * Returns a sequence of changes produced by map component of shared collections.
 * `len` output parameter is used to provide information about number of changes produced.
 *
 * Delta returned from this function should eventually be released using `yevent_keys_destroy`
 * function.
 */
struct YEventKeyChange *yxmltext_event_keys(const struct YXmlTextEvent *e, int *len);

/**
 * Releases memory allocated by the object returned from `yxml_event_keys` and `ymap_event_keys`
 * functions.
 */
void yevent_keys_destroy(struct YEventKeyChange *keys, int len);

/**
 * Returns a value informing what kind of Yrs shared collection given `branch` represents.
 * Returns either 0 when `branch` is null or one of values: `Y_ARRAY`, `Y_TEXT`, `Y_MAP`,
 * `Y_XML_ELEM`, `Y_XML_TEXT`.
 */
char ytype_kind(const Branch *branch);

#endif
