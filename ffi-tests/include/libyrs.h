#ifndef YRS_FFI_H
#define YRS_FFI_H

typedef struct YDoc {} YDoc;
typedef struct YText {} YText;
typedef struct YArray {} YArray;
typedef struct YMap {} YMap;
typedef struct YXmlElement {} YXmlElement;
typedef struct YXmlText {} YXmlText;

typedef struct YTransaction {} YTransaction;
typedef struct YArrayIter {} YArrayIter;
typedef struct YMapIter {} YMapIter;
typedef struct YXmlAttrIter {} YXmlAttrIter;
typedef struct YXmlTreeWalker {} YXmlTreeWalker;


#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * A Yrs document type. Documents are most important units of collaborative resources management.
 * All shared collections live within a scope of their corresponding documents. All updates are
 * generated on per document basis (rather than individual shared type). All operations on shared
 * collections happen via [Transaction], which lifetime is also bound to a document.
 *
 * Document manages so called root types, which are top-level shared types definitions (as opposed
 * to recursively nested types).
 */
typedef YDoc YDoc;

typedef YText YText;

typedef YArray YArray;

typedef YMap YMap;

typedef YXmlElement YXmlElement;

typedef YXmlText YXmlText;

typedef union YOutputContent {
  char flag;
  float num;
  long integer;
  char *str;
  unsigned char *buf;
  struct YOutput *array;
  struct YMapEntry *map;
  YArray *y_array;
  YMap *y_map;
  YText *y_text;
  YXmlElement *y_xmlelem;
  YXmlText *y_xmltext;
} YOutputContent;

/**
 * An output value cell returned from yrs API methods. It describes a various types of data
 * supported by yrs shared data types.
 *
 * Since [YOutput] instances are always created by calling the corresponding yrs API functions,
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
   * - [Y_TEXT] for pointers to [YText] data types.
   * - [Y_ARRAY] for pointers to [YArray] data types.
   * - [Y_MAP] for pointers to [YMap] data types.
   * - [Y_XML_ELEM] for pointers to [YXmlElement] data types.
   * - [Y_XML_TEXT] for pointers to [YXmlText] data types.
   */
  char tag;
  /**
   * Length of the contents stored by a current [YOutput] cell.
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
   * A [YOutput] value representing containing variadic content that can be stored withing map's
   * entry.
   */
  struct YOutput value;
} YMapEntry;

/**
 * A structure representing single attribute of an either [XmlElement] or [XmlText] instance.
 * It consists of attribute name and string, both of which are null-terminated UTF-8 strings.
 */
typedef struct YXmlAttr {
  const char *name;
  const char *value;
} YXmlAttr;

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
 * [YInput] constructor function don't allocate any resources on their own, neither they take
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
   * - [Y_ARRAY] for cells which contents should be used to initialize a [YArray] shared type.
   * - [Y_MAP] for cells which contents should be used to initialize a [YMap] shared type.
   */
  char tag;
  /**
   * Length of the contents stored by current [YInput] cell.
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

typedef YArrayIter YArrayIter;

typedef YMapIter YMapIter;

typedef YXmlAttrIter YXmlAttrIter;

typedef YXmlTreeWalker YXmlTreeWalker;

extern const char Y_JSON_BOOL;

extern const char Y_JSON_NUM;

extern const char Y_JSON_INT;

extern const char Y_JSON_STR;

extern const char Y_JSON_BUF;

extern const char Y_JSON_ARR;

extern const char Y_JSON_MAP;

extern const char Y_JSON_NULL;

extern const char Y_JSON_UNDEF;

extern const char Y_ARRAY;

extern const char Y_MAP;

extern const char Y_TEXT;

extern const char Y_XML_ELEM;

extern const char Y_XML_TEXT;

extern const char Y_TRUE;

extern const char Y_FALSE;

/**
 * Releases all memory-allocated resources bound to given document.
 */
void ydoc_destroy(YDoc *value);

/**
 * Releases all memory-allocated resources bound to given [Text] instance. It doesn't remove the
 * [Text] stored inside of a document itself, but rather only parts of it related to a specific
 * pointer that's a subject of being destroyed.
 */
void ytext_destroy(YText *value);

/**
 * Releases all memory-allocated resources bound to given [Array] instance. It doesn't remove the
 * [Array] stored inside of a document itself, but rather only parts of it related to a specific
 * pointer that's a subject of being destroyed.
 */
void yarray_destroy(YArray *value);

/**
 * Releases all memory-allocated resources bound to given [Map] instance. It doesn't remove the
 * [Map] stored inside of a document itself, but rather only parts of it related to a specific
 * pointer that's a subject of being destroyed.
 */
void ymap_destroy(YMap *value);

/**
 * Releases all memory-allocated resources bound to given [XmlElement] instance. It doesn't remove
 * the [XmlElement] stored inside of a document itself, but rather only parts of it related to
 * a specific pointer that's a subject of being destroyed.
 */
void yxmlelem_destroy(YXmlElement *value);

/**
 * Releases all memory-allocated resources bound to given [XmlText] instance. It doesn't remove
 * the [XmlText] stored inside of a document itself, but rather only parts of it related to
 * a specific pointer that's a subject of being destroyed.
 */
void yxmltext_destroy(YXmlText *value);

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
 * Creates a new [Doc] instance with a specified client `id`. Provided `id` must be unique across
 * all collaborating clients.
 *
 * If two clients share the same `id` and will perform any updates, it will result in unrecoverable
 * document state corruption. The same thing may happen if the client restored document state from
 * snapshot, that didn't contain all of that clients updates that were sent to other peers.
 *
 * Use [ydoc_destroy] in order to release created [Doc] resources.
 */
YDoc *ydoc_new_with_id(unsigned long id);

/**
 * Returns a unique client identifier of this [Doc] instance.
 */
unsigned long ydoc_id(YDoc *doc);

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
 * Gets or creates a new shared [Text] data type instance as a root-level type of a given document.
 * This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
 * compatible string.
 *
 * Use [ytext_destroy] in order to release pointer returned that way - keep in mind that this will
 * not remove [Text] instance from the document itself (once created it'll last for the entire
 * lifecycle of a document).
 */
YText *ytext(YTransaction *txn, const char *name);

/**
 * Gets or creates a new shared [Array] data type instance as a root-level type of a given document.
 * This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
 * compatible string.
 *
 * Use [yarray_destroy] in order to release pointer returned that way - keep in mind that this will
 * not remove [Array] instance from the document itself (once created it'll last for the entire
 * lifecycle of a document).
 */
YArray *yarray(YTransaction *txn, const char *name);

/**
 * Gets or creates a new shared [Map] data type instance as a root-level type of a given document.
 * This structure can later be accessed using its `name`, which must be a null-terminated UTF-8
 * compatible string.
 *
 * Use [ymap_destroy] in order to release pointer returned that way - keep in mind that this will
 * not remove [Map] instance from the document itself (once created it'll last for the entire
 * lifecycle of a document).
 */
YMap *ymap(YTransaction *txn, const char *name);

/**
 * Gets or creates a new shared [XmlElement] data type instance as a root-level type of a given
 * document. This structure can later be accessed using its `name`, which must be a null-terminated
 * UTF-8 compatible string.
 *
 * Use [yxmlelem_destroy] in order to release pointer returned that way - keep in mind that this
 * will not remove [XmlElement] instance from the document itself (once created it'll last for
 * the entire lifecycle of a document).
 */
YXmlElement *yxmlelem(YTransaction *txn, const char *name);

/**
 * Gets or creates a new shared [XmlText] data type instance as a root-level type of a given
 * document. This structure can later be accessed using its `name`, which must be a null-terminated
 * UTF-8 compatible string.
 *
 * Use [yxmltext_destroy] in order to release pointer returned that way - keep in mind that this
 * will not remove [XmlText] instance from the document itself (once created it'll last for
 * the entire lifecycle of a document).
 */
YXmlText *yxmltext(YTransaction *txn, const char *name);

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
 * Applies an diff update (generated by [ytransaction_state_diff_v1]) to a local transaction's
 * document.
 *
 * A length of generated `diff` binary must be passed within a `diff_len` out parameter.
 */
void ytransaction_apply(YTransaction *txn, const unsigned char *diff, int diff_len);

/**
 * Returns the length of the [Text] string content in bytes (without the null terminator character)
 */
int ytext_len(const YText *txt);

/**
 * Returns a null-terminated UTF-8 encoded string content of a current [Text] shared data type.
 *
 * Generated string resources should be released using [ystring_destroy] function.
 */
char *ytext_string(const YText *txt, const YTransaction *txn);

/**
 * Inserts a null-terminated UTF-8 encoded string a a given `index`. `index` value must be between
 * 0 and a length of a [Text] (inclusive, accordingly to [ytext_len] return value), otherwise this
 * function will panic.
 *
 * A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
 * ownership over a passed value - it will be copied and therefore a string parameter must be
 * released by the caller.
 */
void ytext_insert(const YText *txt, YTransaction *txn, int index, const char *value);

/**
 * Removes a range of characters, starting a a given `index`. This range must fit within the bounds
 * of a current [Text], otherwise this function call will fail.
 *
 * An `index` value must be between 0 and the length of a [Text] (exclusive, accordingly to
 * [ytext_len] return value).
 *
 * A `length` must be lower or equal number of bytes (internally [Text] uses UTF-8 encoding) from
 * `index` position to the end of of the string.
 */
void ytext_remove_range(const YText *txt, YTransaction *txn, int index, int length);

/**
 * Returns a number of elements stored within current instance of [Array].
 */
int yarray_len(const YArray *array);

/**
 * Returns a pointer to a [YOutput] value stored at a given `index` of a current [Array].
 * If `index` is outside of the bounds of an array, a null pointer will be returned.
 *
 * A value returned should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yarray_get(const YArray *array, YTransaction *txn, int index);

/**
 * Inserts a range of `items` into current [Array], starting at given `index`. An `items_len`
 * parameter is used to determine the size of `items` array - it can also be used to insert
 * a single element given its pointer.
 *
 * An `index` value must be between 0 and (inclusive) length of a current array (use [yarray_len]
 * to determine its length), otherwise it will panic at runtime.
 *
 * [Array] doesn't take ownership over the inserted `items` data - their contents are being copied
 * into array structure - therefore caller is responsible for freeing all memory associated with
 * input params.
 */
void yarray_insert_range(const YArray *array,
                         YTransaction *txn,
                         int index,
                         const struct YInput *items,
                         int items_len);

/**
 * Removes a `len` of consecutive range of elements from current `array` instance, starting at
 * a given `index`. Range determined by `index` and `len` must fit into boundaries of an array,
 * otherwise it will panic at runtime.
 */
void yarray_remove_range(const YArray *array, YTransaction *txn, int index, int len);

/**
 * Returns an iterator, which can be used to traverse over all elements of an `array` (`array`'s
 * length can be determined using [yarray_len] function).
 *
 * Use [yarray_iter_next] function in order to retrieve a consecutive array elements.
 * Use [yarray_iter_destroy] function in order to close the iterator and release its resources.
 */
YArrayIter *yarray_iter(const YArray *array, const YTransaction *txn);

/**
 * Releases all of an [Array] iterator resources created by calling [yarray_iter].
 */
void yarray_iter_destroy(YArrayIter *iter);

/**
 * Moves current [Array] iterator over to a next element, returning a pointer to it. If an iterator
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
YMapIter *ymap_iter(const YMap *map, const YTransaction *txn);

/**
 * Releases all of an [Map] iterator resources created by calling [ymap_iter].
 */
void ymap_iter_destroy(YMapIter *iter);

/**
 * Moves current [Map] iterator over to a next entry, returning a pointer to it. If an iterator
 * comes to an end of a map, a null pointer will be returned. Yrs maps are unordered and so are
 * their iterators.
 *
 * Returned values should be eventually released using [ymap_entry_destroy] function.
 */
struct YMapEntry *ymap_iter_next(YMapIter *iter);

/**
 * Returns a number of entries stored within a `map`.
 */
int ymap_len(const YMap *map, const YTransaction *txn);

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
void ymap_insert(const YMap *map, YTransaction *txn, const char *key, const struct YInput *value);

/**
 * Removes a `map` entry, given its `key`. Returns `1` if the corresponding entry was successfully
 * removed or `0` if no entry with a provided `key` has been found inside of a `map`.
 *
 * A `key` must be a null-terminated UTF-8 encoded string.
 */
char ymap_remove(const YMap *map, YTransaction *txn, const char *key);

/**
 * Returns a value stored under the provided `key`, or a null pointer if no entry with such `key`
 * has been found in a current `map`. A returned value is allocated by this function and therefore
 * should be eventually released using [youtput_destroy] function.
 *
 * A `key` must be a null-terminated UTF-8 encoded string.
 */
struct YOutput *ymap_get(const YMap *map, const YTransaction *txn, const char *key);

/**
 * Removes all entries from a current `map`.
 */
void ymap_remove_all(const YMap *map, YTransaction *txn);

/**
 * Return a name (or an XML tag) of a current [XmlElement]. Root-level XML nodes use "UNDEFINED" as
 * their tag names.
 *
 * Returned value is a null-terminated UTF-8 string, which must be released using [ystring_destroy]
 * function.
 */
char *yxmlelem_tag(const YXmlElement *xml);

/**
 * Converts current [XmlElement] together with its children and attributes into a flat string
 * representation (no padding) eg. `<UNDEFINED><title key="value">sample text</title></UNDEFINED>`.
 *
 * Returned value is a null-terminated UTF-8 string, which must be released using [ystring_destroy]
 * function.
 */
char *yxmlelem_string(const YXmlElement *xml, const YTransaction *txn);

/**
 * Inserts an XML attribute described using `attr_name` and `attr_value`. If another attribute with
 * the same name already existed, its value will be replaced with a provided one.
 *
 * Both `attr_name` and `attr_value` must be a null-terminated UTF-8 encoded strings. Their
 * contents are being copied, therefore it's up to a function caller to properly release them.
 */
void yxmlelem_insert_attr(const YXmlElement *xml,
                          YTransaction *txn,
                          const char *attr_name,
                          const char *attr_value);

/**
 * Removes an attribute from a current [XmlElement], given its name.
 *
 * An `attr_name`must be a null-terminated UTF-8 encoded string.
 */
void yxmlelem_remove_attr(const YXmlElement *xml, YTransaction *txn, const char *attr_name);

/**
 * Returns the value of a current [XmlElement], given its name, or a null pointer if not attribute
 * with such name has been found. Returned pointer is a null-terminated UTF-8 encoded string, which
 * should be released using [ystring_destroy] function.
 *
 * An `attr_name` must be a null-terminated UTF-8 encoded string.
 */
char *yxmlelem_get_attr(const YXmlElement *xml, const YTransaction *txn, const char *attr_name);

/**
 * Returns an iterator over the [XmlElement] attributes.
 *
 * Use [yxmlattr_iter_next] function in order to retrieve a consecutive (**unordered**) attributes.
 * Use [yxmlattr_iter_destroy] function in order to close the iterator and release its resources.
 */
YXmlAttrIter *yxmlelem_attr_iter(const YXmlElement *xml, const YTransaction *txn);

/**
 * Returns an iterator over the [XmlText] attributes.
 *
 * Use [yxmlattr_iter_next] function in order to retrieve a consecutive (**unordered**) attributes.
 * Use [yxmlattr_iter_destroy] function in order to close the iterator and release its resources.
 */
YXmlAttrIter *yxmltext_attr_iter(const YXmlText *xml, const YTransaction *txn);

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
 * Returns a next sibling of a current [XmlElement], which can be either another [XmlElement]
 * or a [XmlText]. Together with [yxmlelem_first_child] it may be used to iterate over the direct
 * children of an XML node (in order to iterate over the nested XML structure use
 * [yxmlelem_tree_walker]).
 *
 * If current [XmlElement] is the last child, this function returns a null pointer.
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmlelem_next_sibling(const YXmlElement *xml, const YTransaction *txn);

/**
 * Returns a previous sibling of a current [XmlElement], which can be either another [XmlElement]
 * or a [XmlText].
 *
 * If current [XmlElement] is the first child, this function returns a null pointer.
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmlelem_prev_sibling(const YXmlElement *xml, const YTransaction *txn);

/**
 * Returns a next sibling of a current [XmlText], which can be either another [XmlText] or
 * an [XmlElement]. Together with [yxmlelem_first_child] it may be used to iterate over the direct
 * children of an XML node (in order to iterate over the nested XML structure use
 * [yxmlelem_tree_walker]).
 *
 * If current [XmlText] is the last child, this function returns a null pointer.
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmltext_next_sibling(const YXmlText *xml, const YTransaction *txn);

/**
 * Returns a previous sibling of a current [XmlText], which can be either another [XmlText] or
 * an [XmlElement].
 *
 * If current [XmlText] is the first child, this function returns a null pointer.
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmltext_prev_sibling(const YXmlText *xml, const YTransaction *txn);

/**
 * Returns a parent [XmlElement] of a current node, or null pointer when current [XmlElement] is
 * a root-level shared data type.
 *
 * A returned value should be eventually released using [youtput_destroy] function.
 */
YXmlElement *yxmlelem_parent(const YXmlElement *xml, const YTransaction *txn);

/**
 * Returns a number of child nodes (both [XmlElement] and [XmlText]) living under a current XML
 * element. This function doesn't count a recursive nodes, only direct children of a current node.
 */
int yxmlelem_child_len(const YXmlElement *xml, const YTransaction *txn);

/**
 * Returns a first child node of a current [XmlElement], or null pointer if current XML node is
 * empty. Returned value could be either another [XmlElement] or [XmlText].
 *
 * A returned value should be eventually released using [youtput_destroy] function.
 */
struct YOutput *yxmlelem_first_child(const YXmlElement *xml, const YTransaction *txn);

/**
 * Returns an iterator over a nested recursive structure of a current [XmlElement], starting from
 * first of its children. Returned values can be either [XmlElement] or [XmlText] nodes.
 *
 * Use [yxmlelem_tree_walker_next] function in order to iterate over to a next node.
 * Use [yxmlelem_tree_walker_destroy] function to release resources used by the iterator.
 */
YXmlTreeWalker *yxmlelem_tree_walker(const YXmlElement *xml, const YTransaction *txn);

/**
 * Releases resources associated with a current XML tree walker iterator.
 */
void yxmlelem_tree_walker_destroy(YXmlTreeWalker *iter);

/**
 * Moves current `iterator` to a next value (either [XmlElement] or [XmlText]), returning its
 * pointer or a null, if an `iterator` already reached the last successor node.
 *
 * Values returned by this function should be eventually released using [youtput_destroy].
 */
struct YOutput *yxmlelem_tree_walker_next(YXmlTreeWalker *iterator);

/**
 * Inserts an [XmlElement] as a child of a current node at the given `index` and returns its
 * pointer. Node created this way will have a given `name` as its tag (eg. `p` for `<p></p>` node).
 *
 * An `index` value must be between 0 and (inclusive) length of a current XML element (use
 * [yxmlelem_child_len] function to determine its length).
 *
 * A `name` must be a null-terminated UTF-8 encoded string, which will be copied into current
 * document. Therefore `name` should be freed by the function caller.
 */
YXmlElement *yxmlelem_insert_elem(const YXmlElement *xml,
                                  YTransaction *txn,
                                  int index,
                                  const char *name);

/**
 * Inserts an [XmlText] as a child of a current node at the given `index` and returns its
 * pointer.
 *
 * An `index` value must be between 0 and (inclusive) length of a current XML element (use
 * [yxmlelem_child_len] function to determine its length).
 */
YXmlText *yxmlelem_insert_text(const YXmlElement *xml, YTransaction *txn, int index);

/**
 * Removes a consecutive range of child elements (of specified length) from the current
 * [XmlElement], starting at the given `index`. Specified range must fit into boundaries of current
 * XML node children, otherwise this function will panic at runtime.
 */
void yxmlelem_remove_range(const YXmlElement *xml, YTransaction *txn, int index, int len);

/**
 * Returns an XML child node (either a [XmlElement] or [XmlText]) stored at a given `index` of
 * a current [XmlElement]. Returns null pointer if `index` was outside of the bound of current XML
 * node children.
 *
 * Returned value should be eventually released using [youtput_destroy].
 */
const struct YOutput *yxmlelem_get(const YXmlElement *xml, const YTransaction *txn, int index);

/**
 * Returns the length of the [XmlText] string content in bytes (without the null terminator
 * character)
 */
int yxmltext_len(const YXmlText *txt, const YTransaction *txn);

/**
 * Returns a null-terminated UTF-8 encoded string content of a current [XmlText] shared data type.
 *
 * Generated string resources should be released using [ystring_destroy] function.
 */
char *yxmltext_string(const YXmlText *txt, const YTransaction *txn);

/**
 * Inserts a null-terminated UTF-8 encoded string a a given `index`. `index` value must be between
 * 0 and a length of a [XmlText] (inclusive, accordingly to [yxmltext_len] return value), otherwise
 * this function will panic.
 *
 * A `str` parameter must be a null-terminated UTF-8 encoded string. This function doesn't take
 * ownership over a passed value - it will be copied and therefore a string parameter must be
 * released by the caller.
 */
void yxmltext_insert(const YXmlText *txt, YTransaction *txn, int index, const char *str);

/**
 * Removes a range of characters, starting a a given `index`. This range must fit within the bounds
 * of a current [XmlText], otherwise this function call will fail.
 *
 * An `index` value must be between 0 and the length of a [XmlText] (exclusive, accordingly to
 * [yxmltext_len] return value).
 *
 * A `length` must be lower or equal number of bytes (internally [XmlText] uses UTF-8 encoding)
 * from `index` position to the end of of the string.
 */
void yxmltext_remove_range(const YXmlText *txt, YTransaction *txn, int idx, int len);

/**
 * Inserts an XML attribute described using `attr_name` and `attr_value`. If another attribute with
 * the same name already existed, its value will be replaced with a provided one.
 *
 * Both `attr_name` and `attr_value` must be a null-terminated UTF-8 encoded strings. Their
 * contents are being copied, therefore it's up to a function caller to properly release them.
 */
void yxmltext_insert_attr(const YXmlText *txt,
                          YTransaction *txn,
                          const char *attr_name,
                          const char *attr_value);

/**
 * Removes an attribute from a current [XmlText], given its name.
 *
 * An `attr_name`must be a null-terminated UTF-8 encoded string.
 */
void yxmltext_remove_attr(const YXmlText *txt, YTransaction *txn, const char *attr_name);

/**
 * Returns the value of a current [XmlText], given its name, or a null pointer if not attribute
 * with such name has been found. Returned pointer is a null-terminated UTF-8 encoded string, which
 * should be released using [ystring_destroy] function.
 *
 * An `attr_name` must be a null-terminated UTF-8 encoded string.
 */
char *yxmltext_get_attr(const YXmlText *txt, const YTransaction *txn, const char *attr_name);

/**
 * Releases all resources related to a corresponding [YOutput] cell.
 */
void youtput_destroy(struct YOutput *val);

/**
 * Function constructor used to create JSON-like NULL [YInput] cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_null(void);

/**
 * Function constructor used to create JSON-like undefined [YInput] cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_undefined(void);

/**
 * Function constructor used to create JSON-like boolean [YInput] cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_bool(char flag);

/**
 * Function constructor used to create JSON-like 64-bit floating point number [YInput] cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_float(float num);

/**
 * Function constructor used to create JSON-like 64-bit signed integer [YInput] cell.
 * This function doesn't allocate any heap resources.
 */
struct YInput yinput_long(long integer);

/**
 * Function constructor used to create a string [YInput] cell. Provided parameter must be
 * a null-terminated UTF-8 encoded string. This function doesn't allocate any heap resources,
 * and doesn't release any on its own, therefore its up to a caller to free resources once
 * a structure is no longer needed.
 */
struct YInput yinput_string(const char *str);

/**
 * Function constructor used to create a binary [YInput] cell of a specified length.
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_binary(const uint8_t *buf, int len);

/**
 * Function constructor used to create a JSON-like array [YInput] cell of other JSON-like values of
 * a given length. This function doesn't allocate any heap resources and doesn't release any on its
 * own, therefore its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_json_array(struct YInput *values, int len);

/**
 * Function constructor used to create a JSON-like map [YInput] cell of other JSON-like key-value
 * pairs. These pairs are build from corresponding indexes of `keys` and `values`, which must have
 * the same specified length.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_json_map(char **keys, struct YInput *values, int len);

/**
 * Function constructor used to create a nested [YArray] [YInput] cell prefilled with other
 * values of a given length. This function doesn't allocate any heap resources and doesn't release
 * any on its own, therefore its up to a caller to free resources once a structure is no longer
 * needed.
 */
struct YInput yinput_yarray(struct YInput *values, int len);

/**
 * Function constructor used to create a nested [YMap] [YInput] cell prefilled with other key-value
 * pairs. These pairs are build from corresponding indexes of `keys` and `values`, which must have
 * the same specified length.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_ymap(char **keys, struct YInput *values, int len);

/**
 * Function constructor used to create a nested [YText] [YInput] cell prefilled with a specified
 * string, which must be a null-terminated UTF-8 character pointer.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_ytext(char *str);

/**
 * Function constructor used to create a nested [YXmlElement] [YInput] cell with a specified
 * tag name, which must be a null-terminated UTF-8 character pointer.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_yxmlelem(char *name);

/**
 * Function constructor used to create a nested [YXmlText] [YInput] cell prefilled with a specified
 * string, which must be a null-terminated UTF-8 character pointer.
 *
 * This function doesn't allocate any heap resources and doesn't release any on its own, therefore
 * its up to a caller to free resources once a structure is no longer needed.
 */
struct YInput yinput_yxmltext(char *str);

/**
 * Attempts to read the value for a given [YOutput] pointer as a boolean flag, which can be either
 * `1` for truthy case and `0` otherwise. Returns a null pointer in case when a value stored under
 * current [YOutput] cell is not of a boolean type.
 */
const char *youtput_read_bool(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as a 64-bit floating point number.
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not a floating point number.
 */
const float *youtput_read_float(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as a 64-bit signed integer.
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not a signed integer.
 */
const long *youtput_read_long(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as a null-terminated UTF-8 encoded
 * string.
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not a string. Underlying string is released automatically as part of [youtput_destroy]
 * destructor.
 */
char *youtput_read_string(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as a binary payload (which length is
 * stored within `len` filed of a cell itself).
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not a binary type. Underlying binary is released automatically as part of [youtput_destroy]
 * destructor.
 */
const unsigned char *youtput_read_binary(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as a JSON-like array of [YOutput]
 * values (which length is stored within `len` filed of a cell itself).
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not a JSON-like array. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
struct YOutput *youtput_read_json_array(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as a JSON-like map of key-value entries
 * (which length is stored within `len` filed of a cell itself).
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not a JSON-like map. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
struct YMapEntry *youtput_read_json_map(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as an [Array].
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not an [Array]. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
YArray *youtput_read_yarray(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as an [XmlElement].
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not an [XmlElement]. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
YXmlElement *youtput_read_yxmlelem(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as an [Map].
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not an [Map]. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
YMap *youtput_read_ymap(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as an [Text].
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not an [Text]. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
YText *youtput_read_ytext(const struct YOutput *val);

/**
 * Attempts to read the value for a given [YOutput] pointer as an [XmlText].
 *
 * Returns a null pointer in case when a value stored under current [YOutput] cell
 * is not an [XmlText]. Underlying heap resources are released automatically as part of
 * [youtput_destroy] destructor.
 */
YXmlText *youtput_read_yxmltext(const struct YOutput *val);

#endif
