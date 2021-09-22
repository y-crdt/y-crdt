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

typedef struct YOutput {
  char tag;
  int len;
  union YOutputContent value;
} YOutput;

typedef struct YMapEntry {
  const char *key;
  struct YOutput value;
} YMapEntry;

typedef struct YXmlAttr {
  const char *name;
  const char *value;
} YXmlAttr;

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

typedef struct YInput {
  char tag;
  int len;
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

void ydoc_destroy(YDoc *value);

void ytext_destroy(YText *value);

void yarray_destroy(YArray *value);

void ymap_destroy(YMap *value);

void yxmlelem_destroy(YXmlElement *value);

void yxmltext_destroy(YXmlText *value);

void ymap_entry_destroy(struct YMapEntry *value);

void yxmlattr_destroy(struct YXmlAttr *attr);

void ystring_destroy(char *str);

void ybinary_destroy(unsigned char *ptr, int len);

YDoc *ydoc_new(void);

YDoc *ydoc_new_with_id(unsigned long id);

YTransaction *ytransaction_new(YDoc *doc);

void ytransaction_commit(YTransaction *txn);

YText *ytext(YTransaction *txn, const char *name);

YArray *yarray(YTransaction *txn, const char *name);

YMap *ymap(YTransaction *txn, const char *name);

YXmlElement *yxmlelem(YTransaction *txn, const char *name);

YXmlText *yxmltext(YTransaction *txn, const char *name);

unsigned char *ytransaction_state_vector_v1(const YTransaction *txn, int *len);

unsigned char *ytransaction_state_diff_v1(const YTransaction *txn,
                                          const unsigned char *sv,
                                          int sv_len,
                                          int *len);

void ytransaction_apply(YTransaction *txn, const unsigned char *diff, int diff_len);

int ytext_len(const YText *txt);

char *ytext_string(const YText *txt, const YTransaction *txn);

void ytext_insert(const YText *txt, YTransaction *txn, int idx, const char *value);

void ytext_remove_range(const YText *txt, YTransaction *txn, int idx, int len);

int yarray_len(const YArray *array);

struct YOutput *yarray_get(const YArray *array, YTransaction *txn, int idx);

void yarray_insert_range(const YArray *array,
                         YTransaction *txn,
                         int idx,
                         const struct YInput *values,
                         int len);

void yarray_remove_range(const YArray *array, YTransaction *txn, int idx, int len);

YArrayIter *yarray_iter(const YArray *array, const YTransaction *txn);

void yarray_iter_destroy(YArrayIter *iter);

struct YOutput *yarray_iter_next(YArrayIter *iter);

YMapIter *ymap_iter(const YMap *map, const YTransaction *txn);

void ymap_iter_destroy(YMapIter *iter);

struct YMapEntry *ymap_iter_next(YMapIter *iter);

int ymap_len(const YMap *map, const YTransaction *txn);

void ymap_insert(const YMap *map, YTransaction *txn, const char *key, const struct YInput *value);

char ymap_remove(const YMap *map, YTransaction *txn, const char *key);

struct YOutput *ymap_get(const YMap *map, const YTransaction *txn, const char *key);

void ymap_remove_all(const YMap *map, YTransaction *txn);

char *yxmlelem_tag(const YXmlElement *xml);

char *yxmlelem_string(const YXmlElement *xml, const YTransaction *txn);

void yxmlelem_insert_attr(const YXmlElement *xml,
                          YTransaction *txn,
                          const char *attr_name,
                          const char *attr_value);

void yxmlelem_remove_attr(const YXmlElement *xml, YTransaction *txn, const char *attr_name);

char *yxmlelem_get_attr(const YXmlElement *xml, const YTransaction *txn, const char *attr_name);

YXmlAttrIter *yxmlattr_iter(const YXmlElement *xml, const YTransaction *txn);

void yxmlattr_iter_destroy(YXmlAttrIter *iter);

struct YXmlAttr *yxmlattr_iter_next(YXmlAttrIter *iter);

struct YOutput *yxmlelem_next_sibling(const YXmlElement *xml, const YTransaction *txn);

struct YOutput *yxmlelem_prev_sibling(const YXmlElement *xml, const YTransaction *txn);

struct YOutput *yxmltext_next_sibling(const YXmlText *xml, const YTransaction *txn);

struct YOutput *yxmltext_prev_sibling(const YXmlText *xml, const YTransaction *txn);

YXmlElement *yxmlelem_parent(const YXmlElement *xml, const YTransaction *txn);

int yxmlelem_child_len(const YXmlElement *xml, const YTransaction *txn);

struct YOutput *yxmlelem_first_child(const YXmlElement *xml, const YTransaction *txn);

YXmlTreeWalker *yxmlelem_tree_walker(const YXmlElement *xml, const YTransaction *txn);

void yxmlelem_tree_walker_destroy(YXmlTreeWalker *iter);

struct YOutput *yxmlelem_tree_walker_next(YXmlTreeWalker *iter);

YXmlElement *yxmlelem_insert_elem(const YXmlElement *xml,
                                  YTransaction *txn,
                                  int idx,
                                  const char *name);

YXmlText *yxmlelem_insert_text(const YXmlElement *xml, YTransaction *txn, int idx);

void yxmlelem_remove_range(const YXmlElement *xml, YTransaction *txn, int idx, int len);

const struct YOutput *yxmlelem_get(const YXmlElement *xml, const YTransaction *txn, int idx);

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

void youtput_destroy(struct YOutput *val);

struct YInput yinput_null(void);

struct YInput yinput_undefined(void);

struct YInput yinput_bool(char flag);

struct YInput yinput_float(float num);

struct YInput yinput_long(long integer);

struct YInput yinput_string(const char *str);

struct YInput yinput_binary(const uint8_t *buf, int len);

struct YInput yinput_json_array(struct YInput *values, int len);

struct YInput yinput_json_map(char **keys, struct YInput *values, int len);

struct YInput yinput_yarray(struct YInput *values, int len);

struct YInput yinput_ymap(char **keys, struct YInput *values, int len);

struct YInput yinput_ytext(char *str);

struct YInput yinput_yxmlelem(struct YInput *xml_children, int len);

struct YInput yinput_yxmltext(char *str);

const char *youtput_read_bool(const struct YOutput *val);

const float *youtput_read_float(const struct YOutput *val);

const long *youtput_read_long(const struct YOutput *val);

char *youtput_read_string(const struct YOutput *val);

const unsigned char *youtput_read_binary(const struct YOutput *val);

struct YOutput *youtput_read_json_array(const struct YOutput *val);

struct YMapEntry *youtput_read_json_map(const struct YOutput *val);

YArray *youtput_read_yarray(const struct YOutput *val);

YXmlElement *youtput_read_yxmlelem(const struct YOutput *val);

YMap *youtput_read_ymap(const struct YOutput *val);

YText *youtput_read_ytext(const struct YOutput *val);

YXmlText *youtput_read_yxmltext(const struct YOutput *val);

#endif
