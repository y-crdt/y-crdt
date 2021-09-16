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

typedef YDoc YDoc;

typedef YText YText;

typedef YArray YArray;

typedef YMap YMap;

typedef YXmlElement YXmlElement;

typedef YXmlText YXmlText;

typedef union YValContent {
  uint8_t flag;
  float num;
  long integer;
  char *str;
  unsigned char *buf;
  struct YVal **array;
  struct YMapEntry **map;
  YArray *y_array;
  YMap *y_map;
  YText *y_text;
  YXmlElement *y_xml_elem;
  YXmlText *y_xml_text;
} YValContent;

typedef struct YVal {
  char tag;
  uint8_t prelim;
  int len;
  union YValContent value;
} YVal;

typedef struct YMapEntry {
  const char *key;
  const struct YVal *value;
} YMapEntry;

typedef struct YXmlAttr {
  const char *name;
  const char *value;
} YXmlAttr;

typedef YTransaction YTransaction;

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

void ydoc_destroy(YDoc *value);

void ytext_destroy(YText *value);

void yarray_destroy(YArray *value);

void ymap_destroy(YMap *value);

void yxml_destroy(YXmlElement *value);

void yxml_text_destroy(YXmlText *value);

void ymap_entry_destroy(struct YMapEntry *value);

void yxml_attr_destroy(struct YXmlAttr *attr);

void ystr_destroy(char *str);

void ybinary_destroy(unsigned char *ptr, int len);

YDoc *ydoc_new(void);

YDoc *ydoc_new_with_id(unsigned long id);

YTransaction *ytxn_new(YDoc *doc);

void ytxn_commit(YTransaction *txn);

YText *ytxn_text(YTransaction *txn, const char *name);

YArray *ytxn_array(YTransaction *txn, const char *name);

YMap *ytxn_map(YTransaction *txn, const char *name);

YXmlElement *ytxn_xml_elem(YTransaction *txn, const char *name);

YXmlText *ytxn_xml_text(YTransaction *txn, const char *name);

const unsigned char *ytxn_state_vector_v1(const YTransaction *txn, int *len);

const unsigned char *ytxn_state_diff_v1(const YTransaction *txn,
                                        const unsigned char *sv,
                                        int sv_len,
                                        int *len);

void ytxn_apply(YTransaction *txn, const uint8_t *diff, int diff_len);

int ytext_len(const YText *txt);

char *ytext_string(const YText *txt, const YTransaction *txn);

void ytext_insert(const YText *txt, YTransaction *txn, int idx, const char *value);

void ytext_remove_range(const YText *txt, YTransaction *txn, int idx, int len);

int yarray_len(const YArray *array);

const struct YVal *yarray_get(const YArray *array, YTransaction *txn, int idx);

void yarray_insert_range(const YArray *array,
                         YTransaction *txn,
                         int idx,
                         const struct YVal *const *values,
                         int len);

void yarray_remove_range(const YArray *array, YTransaction *txn, int idx, int len);

YArrayIter *yarray_iter(const YArray *array, const YTransaction *txn);

void yarray_iter_destroy(YArrayIter *iter);

struct YVal *yarray_iter_next(YArrayIter *iter);

YMapIter *ymap_iter(const YMap *map, const YTransaction *txn);

void ymap_iter_destroy(YMapIter *iter);

struct YMapEntry *ymap_iter_next(YMapIter *iter);

int ymap_len(const YMap *map, const YTransaction *txn);

const struct YMapEntry *ymap_insert(const YMap *map,
                                    YTransaction *txn,
                                    const char *key,
                                    const struct YVal *value);

const struct YVal *ymap_remove(const YMap *map, YTransaction *txn, const char *key);

const struct YVal *ymap_get(const YMap *map, const YTransaction *txn, const char *key);

void ymap_remove_all(const YMap *map, YTransaction *txn);

char *yxml_tag(const YXmlElement *xml);

char *yxml_string(const YXmlElement *xml, const YTransaction *txn);

void yxml_insert_attr(const YXmlElement *xml,
                      YTransaction *txn,
                      const char *attr_name,
                      const char *attr_value);

void yxml_remove_attr(const YXmlElement *xml, YTransaction *txn, const char *attr_name);

char *yxml_get_attr(const YXmlElement *xml, const YTransaction *txn, const char *attr_name);

int yxml_attr_len(const YXmlElement *xml, const YTransaction *txn);

YXmlAttrIter *yxml_attr_iter(const YXmlElement *xml, const YTransaction *txn);

void yxml_attr_iter_destroy(YXmlAttrIter *iter);

struct YXmlAttr *yxml_attr_iter_next(YXmlAttrIter *iter);

const struct YVal *yxml_next_sibling(const YXmlElement *xml, const YTransaction *txn);

const struct YVal *yxml_prev_sibling(const YXmlElement *xml, const YTransaction *txn);

const struct YVal *yxml_parent(const YXmlElement *xml, const YTransaction *txn);

int yxml_child_len(const YXmlElement *xml, const YTransaction *txn);

YXmlTreeWalker *yxml_tree_walker(const YXmlElement *xml, const YTransaction *txn);

void yxml_tree_walker_destroy(YXmlTreeWalker *iter);

const struct YVal *yxml_tree_walker_next(YXmlTreeWalker *iter);

YXmlElement *yxml_insert_elem(const YXmlElement *xml, YTransaction *txn, int idx, const char *name);

YXmlText *yxml_insert_text(const YXmlElement *xml, YTransaction *txn, int idx);

void yxml_remove_range(const YXmlElement *xml, YTransaction *txn, int idx, int len);

const struct YVal *yxml_get(const YXmlElement *xml, const YTransaction *txn, int idx);

void yxmltext_destroy(YXmlText *txt);

int yxmltext_len(const YXmlText *txt, const YTransaction *txn);

char *yxmltext_string(const YXmlText *txt, const YTransaction *txn);

void yxmltext_insert(const YXmlText *txt, YTransaction *txn, int idx, const char *str);

void yxmltext_remove_range(const YXmlText *txt, YTransaction *txn, int idx, int len);

void yxmltext_insert_attr(const YXmlText *txt,
                          YTransaction *txn,
                          const char *attr_name,
                          const char *attr_value);

void yxmltext_remove_attr(const YXmlText *txt, YTransaction *txn, const char *attr_name);

char *yxmltext_get_attr(const YXmlText *txt, const YTransaction *txn, const char *attr_name);

YXmlAttrIter *yxmltext_attrs_iter(const YXmlText *txt, const YTransaction *txn);

void yval_destroy(struct YVal *val);

const struct YVal *yval_null(void);

struct YVal *yval_undef(void);

struct YVal *yval_bool(uint8_t flag);

struct YVal *yval_float(float num);

struct YVal *yval_long(long integer);

struct YVal *yval_str(const char *str);

struct YVal *yval_buf(const uint8_t *buf, int len);

struct YVal *yval_json_array(struct YVal **json_array, int len);

struct YVal *yval_json_map(struct YMapEntry **json_map, int len);

struct YVal *yval_yarray(struct YVal **array, int len);

struct YVal *yval_ymap(struct YMapEntry **map, int len);

struct YVal *yval_yxml_elem(struct YVal **array, int len);

struct YVal *yval_yxml_text(void);

char yval_is_json(const struct YVal *val);

const unsigned char *yval_read_bool(const struct YVal *val);

const float *yval_read_float(const struct YVal *val);

const long *yval_read_long(const struct YVal *val);

char *yval_read_str(const struct YVal *val);

const uint8_t *yval_read_buf(const struct YVal *val);

struct YVal **yval_read_json_array(const struct YVal *val);

struct YMapEntry **yval_read_json_map(const struct YVal *val);

YArray *yval_read_yarray(const struct YVal *val);

YMap *yval_read_ymap(const struct YVal *val);

YText *yval_read_ytext(const struct YVal *val);

const YXmlText *yval_read_yxml_text(const struct YVal *val);

const YXmlElement *yval_read_yxml_elem(const struct YVal *val);

#endif
