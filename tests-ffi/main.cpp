#define DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN

#include <stdio.h>
#include <string.h>
#include "include/doctest.h"

extern "C" {
    #include "include/libyrs.h"
};

YDoc* ydoc_new_with_id(int id) {
    YOptions o;
    o.encoding = Y_OFFSET_UTF16;
    o.id = id;
    o.skip_gc = 0;

    return ydoc_new_with_options(o);
}

TEST_CASE("Update exchange basic") {
    // init
    YDoc* d1 = ydoc_new_with_id(1);
    YTransaction* t1 = ytransaction_new(d1);
    YText* txt1 = ytext(t1, "test");

    YDoc* d2 = ydoc_new_with_id(2);
    YTransaction* t2 = ytransaction_new(d2);
    YText* txt2 = ytext(t2, "test");

    // insert data at the same position on both peer texts
    ytext_insert(txt1, t1, 0, "world", NULL);
    ytext_insert(txt2, t2, 0, "hello ", NULL);

    // exchange updates
    int sv1_len = 0;
    unsigned char* sv1 = ytransaction_state_vector_v1(t1, &sv1_len);

    int sv2_len = 0;
    unsigned char* sv2 = ytransaction_state_vector_v1(t2, &sv2_len);

    int u1_len = 0;
    unsigned char* u1 = ytransaction_state_diff_v1(t1, sv2, sv2_len, &u1_len);

    int u2_len = 0;
    unsigned char* u2 = ytransaction_state_diff_v1(t2, sv1, sv1_len, &u2_len);

    ybinary_destroy(sv1, sv1_len);
    ybinary_destroy(sv2, sv2_len);

    // apply updates
    ytransaction_apply(t1, u2, u2_len);
    ytransaction_apply(t2, u1, u1_len);

    ybinary_destroy(u1, u1_len);
    ybinary_destroy(u2, u2_len);

    // make sure both peers produce the same output
    char* str1 = ytext_string(txt1, t1);
    char* str2 = ytext_string(txt2, t2);

    REQUIRE(!strcmp(str1, str2));

    ystring_destroy(str1);
    ystring_destroy(str2);

    ytext_destroy(txt2);
    ytransaction_commit(t2);
    ydoc_destroy(d2);

    ytext_destroy(txt1);
    ytransaction_commit(t1);
    ydoc_destroy(d1);
}

TEST_CASE("YText basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    YText* txt = ytext(txn, "test");

    ytext_insert(txt, txn, 0, "hello", NULL);
    ytext_insert(txt, txn, 5, " world", NULL);
    ytext_remove_range(txt, txn, 0, 6);

    REQUIRE_EQ(ytext_len(txt), 5);

    char* str = ytext_string(txt, txn);
    REQUIRE(!strcmp(str, "world"));

    ystring_destroy(str);
    ytext_destroy(txt);
    ytransaction_commit(txn);
    ydoc_destroy(doc);
}

TEST_CASE("YArray basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    YArray* arr = yarray(txn, "test");

    YInput* nested = (YInput*)malloc(2 * sizeof(YInput));
    nested[0] = yinput_float(0.5);
    nested[1] = yinput_bool(1);
    YInput nested_array = yinput_yarray(nested, 2);

    const int ARG_LEN = 3;

    YInput* args = (YInput*)malloc(ARG_LEN * sizeof(YInput));
    args[0] = nested_array;
    args[1] = yinput_string("hello");
    args[2] = yinput_long(123);

    yarray_insert_range(arr, txn, 0, args, ARG_LEN); //state after: [ YArray([0.5, true]), 'hello', 123]

    free(nested);
    free(args);

    yarray_remove_range(arr, txn, 1, 1); //state after: [ YArray([0.5, true]), 123 ]

    REQUIRE_EQ(yarray_len(arr), 2);

    YArrayIter* i = yarray_iter(arr, txn);

    // first outer YArray element should be another YArray([0.5, true])
    YOutput* curr = yarray_iter_next(i);
    YArray* a = youtput_read_yarray(curr);
    REQUIRE_EQ(yarray_len(a), 2);

    // read 0th element of inner YArray
    YOutput* elem = yarray_get(a, txn, 0);
    REQUIRE_EQ(*youtput_read_float(elem), 0.5);
    youtput_destroy(elem);

    // read 1st element of inner YArray
    elem = yarray_get(a, txn, 1);
    REQUIRE_EQ(*youtput_read_bool(elem), 1); // in C we use 1 to mark TRUE
    youtput_destroy(elem);
    youtput_destroy(curr);

    // second outer YArray element should be 123
    curr = yarray_iter_next(i);
    REQUIRE_EQ(*youtput_read_long(curr), 123);
    youtput_destroy(curr);

    curr = yarray_iter_next(i);
    REQUIRE(curr == NULL);

    yarray_iter_destroy(i);
    yarray_destroy(arr);
    ytransaction_commit(txn);
    ydoc_destroy(doc);
}

TEST_CASE("YMap basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    YMap* map = ymap(txn, "test");

    // insert 'a' => 'value'
    YInput a = yinput_string("value");

    ymap_insert(map, txn, "a", &a);

    // insert 'b' -> [11,22]
    YInput* array = (YInput*) malloc(2 * sizeof(YInput));
    array[0] = yinput_long(11);
    array[1] = yinput_long(22);
    YInput b = yinput_json_array(array, 2);

    ymap_insert(map, txn, "b", &b);
    free(array);

    REQUIRE_EQ(ymap_len(map, txn), 2);

    // iterate over entries
    YMapIter* i = ymap_iter(map, txn);
    YMapEntry* curr;

    YMapEntry** acc = (YMapEntry**)malloc(2 * sizeof(YMapEntry*));
    acc[0] = ymap_iter_next(i);
    acc[1] = ymap_iter_next(i);

    curr = ymap_iter_next(i);
    REQUIRE(curr == NULL);

    ymap_iter_destroy(i);

    for (int i = 0; i < 2; i++) {
        curr = acc[i];
        switch (curr->key[0]) {
            case 'a': {
                REQUIRE(!strcmp(curr->key, "a"));
                REQUIRE(!strcmp(youtput_read_string(&curr->value), "value"));
                break;
            }
            case 'b': {
                REQUIRE(!strcmp(curr->key, "b"));
                REQUIRE_EQ(curr->value.len, 2);
                YOutput* output = youtput_read_json_array(&curr->value);
                YOutput* fst = &output[0];
                YOutput* snd = &output[1];
                REQUIRE_EQ(*youtput_read_long(fst), 11);
                REQUIRE_EQ(*youtput_read_long(snd), 22);
                break;
            }
            default: {
                FAIL("Unrecognized case: ", curr->key);
                break;
            }
        }
        ymap_entry_destroy(curr);
    }

    free(acc);

    // remove 'a' twice - second attempt should return null
    char removed = ymap_remove(map, txn, "a");
    REQUIRE_EQ(removed, 1);

    removed = ymap_remove(map, txn, "a");
    REQUIRE_EQ(removed, 0);

    // get 'b' and read its contents
    YOutput* out = ymap_get(map, txn, "b");
    YOutput* output = youtput_read_json_array(out);
    REQUIRE_EQ(out->len, 2);
    REQUIRE_EQ(*youtput_read_long(&output[0]), 11);
    REQUIRE_EQ(*youtput_read_long(&output[1]), 22);
    youtput_destroy(out);

    // clear map
    ymap_remove_all(map, txn);
    REQUIRE_EQ(ymap_len(map, txn), 0);

    ymap_destroy(map);
    ytransaction_commit(txn);
    ydoc_destroy(doc);
}

TEST_CASE("YXmlElement basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    YXmlElement* xml = yxmlelem(txn, "test");

    // XML attributes API
    yxmlelem_insert_attr(xml, txn, "key1", "value1");
    yxmlelem_insert_attr(xml, txn, "key2", "value2");

    YXmlAttrIter* i = yxmlelem_attr_iter(xml, txn);
    YXmlAttr* attr;

    YXmlAttr** attrs = (YXmlAttr**)malloc(2 * sizeof(YXmlAttr*));
    attrs[0] = yxmlattr_iter_next(i);
    attrs[1] = yxmlattr_iter_next(i);

    attr = yxmlattr_iter_next(i);
    REQUIRE(attr == NULL);
    yxmlattr_destroy(attr);

    for (int j = 0; j < 2; ++j) {
        attr = attrs[j];
        switch (attr->name[3]) {
            case '1': {
                REQUIRE(!strcmp(attr->name, "key1"));
                REQUIRE(!strcmp(attr->value, "value1"));
                break;
            }
            case '2': {
                REQUIRE(!strcmp(attr->name, "key2"));
                REQUIRE(!strcmp(attr->value, "value2"));
                break;
            }
            default: {
                FAIL("Unrecognized attribute name: ", attr->name);
                break;
            }
        }
        yxmlattr_destroy(attr);
    }

    // XML children API
    YXmlElement* inner = yxmlelem_insert_elem(xml, txn, 0, "p");
    YXmlText* inner_txt = yxmlelem_insert_text(inner, txn, 0);
    yxmltext_insert(inner_txt, txn, 0, "hello", NULL);

    REQUIRE_EQ(yxmlelem_child_len(xml, txn), 1);

    YXmlText* txt = yxmlelem_insert_text(xml, txn, 1);
    yxmltext_insert(txt, txn, 0, "world", NULL);

    // check tag names
    char* tag = yxmlelem_tag(inner);
    REQUIRE(!strcmp(tag, "p"));
    ystring_destroy(tag);

    tag = yxmlelem_tag(xml);
    REQUIRE(!strcmp(tag, "UNDEFINED"));
    ystring_destroy(tag);

    // check parents
    YXmlElement* parent = yxmlelem_parent(inner, txn);
    tag = yxmlelem_tag(parent);
    REQUIRE(!strcmp(tag, "UNDEFINED"));
    ystring_destroy(tag);
    yxmlelem_destroy(parent);

    parent = yxmlelem_parent(xml, txn);
    REQUIRE(parent == NULL);

    // check children traversal
    YOutput* curr = yxmlelem_first_child(xml, txn);
    YXmlElement* first = youtput_read_yxmlelem(curr);
    REQUIRE(yxmlelem_prev_sibling(first, txn) == NULL);
    char* str = yxmlelem_string(first, txn);
    REQUIRE(!strcmp(str, "<p>hello</p>"));
    ystring_destroy(str);

    YOutput* next = yxmlelem_next_sibling(first, txn);
    youtput_destroy(curr);
    YXmlText* second = youtput_read_yxmltext(next);
    REQUIRE(yxmltext_next_sibling(second, txn) == NULL);
    str = yxmltext_string(second, txn);
    REQUIRE(!(strcmp(str, "world")));
    ystring_destroy(str);

    // check tree walker - expected order:
    // - p
    // - hello
    // - world
    YXmlTreeWalker* w = yxmlelem_tree_walker(xml, txn);
    YXmlElement* e;

    curr = yxmlelem_tree_walker_next(w);
    e = youtput_read_yxmlelem(curr);
    str = yxmlelem_string(e, txn);
    REQUIRE(!strcmp(str, "<p>hello</p>"));
    ystring_destroy(str);
    youtput_destroy(curr);

    curr = yxmlelem_tree_walker_next(w);
    YXmlText* t = youtput_read_yxmltext(curr);
    str = yxmltext_string(t, txn);
    REQUIRE(!strcmp(str, "hello"));
    ystring_destroy(str);
    youtput_destroy(curr);

    curr = yxmlelem_tree_walker_next(w);
    t = youtput_read_yxmltext(curr);
    str = yxmltext_string(t, txn);
    REQUIRE(!strcmp(str, "world"));
    ystring_destroy(str);
    youtput_destroy(curr);

    curr = yxmlelem_tree_walker_next(w);
    REQUIRE(curr == NULL);

    yxmlelem_tree_walker_destroy(w);

    yxmltext_destroy(inner_txt);
    yxmltext_destroy(txt);
    yxmlelem_destroy(inner);

    yxmlelem_destroy(xml);
    ytransaction_commit(txn);
    ydoc_destroy(doc);
}

typedef struct YTextEventTest {
    int delta_len;
    YDelta* delta;
    YText* target;
} YEventTest;

YTextEventTest* ytext_event_test_new() {
    YTextEventTest* t = (YTextEventTest*)malloc(sizeof(YTextEventTest));
    t->target = NULL;
    t->delta = NULL;
    t->delta_len = 0;

    return t;
}

void ytext_test_observe(void* state, const YTextEvent* e) {
    YTextEventTest* t = (YTextEventTest*) state;
    t->target = ytext_event_target(e);
    t->delta = ytext_event_delta(e, &t->delta_len);
}

void ytext_test_clean(YTextEventTest* t) {
    ytext_destroy(t->target);
    ytext_delta_destroy(t->delta, t->delta_len);
    t->target = NULL;
    t->delta = NULL;
}

TEST_CASE("YText observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    YText* txt = ytext(txn, "test");

    YTextEventTest* t = ytext_event_test_new();
    unsigned int sub = ytext_observe(txt, (void*)t, &ytext_test_observe);

    // insert initial data to an empty YText
    ytext_insert(txt, txn, 0, "abcd", NULL);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 1);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[0].insert->len == 4);

    // remove 2 chars from the middle
    ytext_test_clean(t);
    txn = ytransaction_new(doc);
    ytext_remove_range(txt, txn, 1, 2);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 2);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_RETAIN);
    REQUIRE(t->delta[0].len == 1);
    REQUIRE(t->delta[1].tag == Y_EVENT_CHANGE_DELETE);
    REQUIRE(t->delta[1].len == 2);

    // insert new item in the middle
    ytext_test_clean(t);
    txn = ytransaction_new(doc);
    ytext_insert(txt, txn, 1, "e", NULL);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 2);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_RETAIN);
    REQUIRE(t->delta[0].len == 1);
    REQUIRE(t->delta[1].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[1].len == 1);

    // free the observer and make sure that callback is no longer called
    ytext_test_clean(t);
    ytext_unobserve(txt, sub);

    txn = ytransaction_new(doc);
    ytext_insert(txt, txn, 1, "fgh", NULL);
    ytransaction_commit(txn);

    REQUIRE(t->target == NULL);
    REQUIRE(t->delta == NULL);

    free(t);
    ytext_destroy(txt);
    ydoc_destroy(doc);
}

typedef struct YArrayEventTest {
    int delta_len;
    YEventChange* delta;
    YArray* target;
} YArrayEventTest;

YArrayEventTest* yarray_event_test_new() {
    YArrayEventTest* t = (YArrayEventTest*)malloc(sizeof(YArrayEventTest));
    t->target = NULL;
    t->delta = NULL;
    t->delta_len = 0;

    return t;
}

void yarray_test_observe(void* state, const YArrayEvent* e) {
    YArrayEventTest* t = (YArrayEventTest*) state;
    t->target = yarray_event_target(e);
    t->delta = yarray_event_delta(e, &t->delta_len);
}

void yarray_test_clean(YArrayEventTest* t) {
    yarray_destroy(t->target);
    yevent_delta_destroy(t->delta, t->delta_len);
    t->target = NULL;
    t->delta = NULL;
}

TEST_CASE("YArray observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    YArray* array = yarray(txn, "test");

    YArrayEventTest* t = yarray_event_test_new();
    unsigned int sub = yarray_observe(array, (void*)t, &yarray_test_observe);

    // insert initial data to an empty YArray
    YInput* i = (YInput*)malloc(4 * sizeof(YInput));
    i[0] = yinput_long(1);
    i[1] = yinput_long(2);
    i[2] = yinput_long(3);
    i[3] = yinput_long(4);

    yarray_insert_range(array, txn, 0, i, 4);
    ytransaction_commit(txn);
    free(i);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 1);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[0].len == 4);

    // remove 2 items from the middle
    yarray_test_clean(t);
    txn = ytransaction_new(doc);
    yarray_remove_range(array, txn, 1, 2);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 2);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_RETAIN);
    REQUIRE(t->delta[0].len == 1);
    REQUIRE(t->delta[1].tag == Y_EVENT_CHANGE_DELETE);
    REQUIRE(t->delta[1].len == 2);

    yarray_test_clean(t);

    // insert new item in the middle
    i = (YInput*)malloc(1 * sizeof(YInput));
    i[0] = yinput_long(5);

    txn = ytransaction_new(doc);
    yarray_insert_range(array, txn, 1, i, 1);
    ytransaction_commit(txn);
    free(i);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 2);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_RETAIN);
    REQUIRE(t->delta[0].len == 1);
    REQUIRE(t->delta[1].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[1].len == 1);

    yarray_test_clean(t);

    // free the observer and make sure that callback is no longer called
    yarray_unobserve(array, sub);

    i = (YInput*)malloc(1 * sizeof(YInput));
    i[0] = yinput_long(5);

    txn = ytransaction_new(doc);
    yarray_insert_range(array, txn, 1, i, 1);
    ytransaction_commit(txn);
    free(i);

    REQUIRE(t->target == NULL);
    REQUIRE(t->delta == NULL);

    free(t);
    yarray_destroy(array);
    ydoc_destroy(doc);
}

typedef struct YMapEventTest {
    int keys_len;
    YEventKeyChange * keys;
    YMap* target;
} YMapEventTest;

YMapEventTest* ymap_event_test_new() {
    YMapEventTest* t = (YMapEventTest*)malloc(sizeof(YMapEventTest));
    t->target = NULL;
    t->keys = NULL;
    t->keys_len = 0;

    return t;
}

void ymap_test_observe(void* state, const YMapEvent* e) {
    YMapEventTest* t = (YMapEventTest*) state;
    t->target = ymap_event_target(e);
    t->keys = ymap_event_keys(e, &t->keys_len);
}

void ymap_test_clean(YMapEventTest* t) {
    ymap_destroy(t->target);
    yevent_keys_destroy(t->keys, t->keys_len);
    t->target = NULL;
    t->keys = NULL;
}

TEST_CASE("YMap observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    YMap* map = ymap(txn, "test");

    YMapEventTest* t = ymap_event_test_new();
    unsigned int sub = ymap_observe(map, (void*)t, &ymap_test_observe);

    // insert initial data to an empty YMap
    YInput i1 = yinput_string("value1");
    YInput i2 = yinput_long(2);
    ymap_insert(map, txn, "key1", &i1);
    ymap_insert(map, txn, "key2", &i2);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->keys_len == 2);

    for (int i = 0; i < t->keys_len; i++) {
        YEventKeyChange e = t->keys[i];
        switch (e.key[3]) {
            case '1': {
                REQUIRE(e.tag == Y_EVENT_KEY_CHANGE_ADD);
                REQUIRE(!strcmp(e.key, "key1"));
                REQUIRE(e.old_value == NULL);
                REQUIRE(!strcmp(youtput_read_string(e.new_value), "value1"));
                break;
            }
            case '2': {
                REQUIRE(e.tag == Y_EVENT_KEY_CHANGE_ADD);
                REQUIRE(!strcmp(e.key, "key2"));
                REQUIRE(e.old_value == NULL);
                REQUIRE(*youtput_read_long(e.new_value) == 2);
                break;
            }
            default: FAIL("unrecognized case");
        }
    }

    // remove an entry and update another on
    ymap_test_clean(t);
    txn = ytransaction_new(doc);
    ymap_remove(map, txn, "key1");
    i2 = yinput_string("value2");
    ymap_insert(map, txn, "key2", &i2);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->keys_len == 2);

    for (int i = 0; i < t->keys_len; i++) {
        YEventKeyChange e = t->keys[i];
        switch (e.key[3]) {
            case '1': {
                REQUIRE(e.tag == Y_EVENT_KEY_CHANGE_DELETE);
                REQUIRE(!strcmp(e.key, "key1"));
                REQUIRE(!strcmp(youtput_read_string(e.old_value), "value1"));
                REQUIRE(e.new_value == NULL);
                break;
            }
            case '2': {
                REQUIRE(e.tag == Y_EVENT_KEY_CHANGE_UPDATE);
                REQUIRE(!strcmp(e.key, "key2"));
                REQUIRE((*youtput_read_long(e.old_value)) == 2);
                REQUIRE(!strcmp(youtput_read_string(e.new_value), "value2"));
                break;
            }
            default: FAIL("unrecognized case");
        }
    }

    // free the observer and make sure that callback is no longer called
    ymap_test_clean(t);
    ymap_unobserve(map, sub);
    txn = ytransaction_new(doc);
    ymap_remove(map, txn, "key2");
    ytransaction_commit(txn);

    REQUIRE(t->target == NULL);
    REQUIRE(t->keys == NULL);

    ymap_destroy(map);
    ydoc_destroy(doc);
}

TEST_CASE("YXmlText observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    YXmlText* txt = yxmltext(txn, "test");

    YTextEventTest* t = ytext_event_test_new();
    unsigned int sub = yxmltext_observe(txt, (void*)t, &ytext_test_observe);

    // insert initial data to an empty YText
    yxmltext_insert(txt, txn, 0, "abcd", NULL);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 1);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[0].insert->len == 4);

    // remove 2 chars from the middle
    ytext_test_clean(t);
    txn = ytransaction_new(doc);
    yxmltext_remove_range(txt, txn, 1, 2);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 2);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_RETAIN);
    REQUIRE(t->delta[0].len == 1);
    REQUIRE(t->delta[1].tag == Y_EVENT_CHANGE_DELETE);
    REQUIRE(t->delta[1].len == 2);

    // insert new item in the middle
    ytext_test_clean(t);
    txn = ytransaction_new(doc);
    yxmltext_insert(txt, txn, 1, "e", NULL);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 2);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_RETAIN);
    REQUIRE(t->delta[0].len == 1);
    REQUIRE(t->delta[1].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[1].len == 1);

    // free the observer and make sure that callback is no longer called
    ytext_test_clean(t);
    yxmltext_unobserve(txt, sub);

    txn = ytransaction_new(doc);
    yxmltext_insert(txt, txn, 1, "fgh", NULL);
    ytransaction_commit(txn);

    REQUIRE(t->target == NULL);
    REQUIRE(t->delta == NULL);

    free(t);
    yxmltext_destroy(txt);
    ydoc_destroy(doc);
}

typedef struct YXmlEventTest {
    YXmlElement * target;
    int keys_len;
    YEventKeyChange * keys;
    int delta_len;
    YEventChange* delta;
} YXmlEventTest;

YXmlEventTest* yxml_event_test_new() {
    YXmlEventTest* t = (YXmlEventTest*)malloc(sizeof(YXmlEventTest));
    t->target = NULL;
    t->keys = NULL;
    t->keys_len = 0;
    t->delta_len = 0;
    t->delta = NULL;

    return t;
}

void yxml_test_observe(void* state, const YXmlEvent* e) {
    YXmlEventTest* t = (YXmlEventTest*) state;
    t->target = yxml_event_target(e);
    t->keys = yxml_event_keys(e, &t->keys_len);
    t->delta = yxml_event_delta(e, &t->delta_len);
}

void yxml_test_clean(YXmlEventTest* t) {
    yxmlelem_destroy(t->target);
    yevent_keys_destroy(t->keys, t->keys_len);
    yevent_delta_destroy(t->delta, t->delta_len);
    t->target = NULL;
    t->keys = NULL;
    t->delta = NULL;
    t->delta_len = 0;
    t->keys_len = 0;
}

TEST_CASE("YXmlElement observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    YXmlElement* xml = yxmlelem(txn, "test");

    YXmlEventTest* t = yxml_event_test_new();
    unsigned int sub = yxmlelem_observe(xml, (void*)t, &yxml_test_observe);

    // insert initial attributes
    yxmlelem_insert_attr(xml, txn, "attr1", "value1");
    yxmlelem_insert_attr(xml, txn, "attr2", "value2");
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->keys_len == 2);

    for (int i = 0; i < t->keys_len; i++) {
        YEventKeyChange e = t->keys[i];
        switch (e.key[4]) {
            case '1': {
                REQUIRE(e.tag == Y_EVENT_KEY_CHANGE_ADD);
                REQUIRE(!strcmp(e.key, "attr1"));
                REQUIRE(e.old_value == NULL);
                REQUIRE(!strcmp(youtput_read_string(e.new_value), "value1"));
                break;
            }
            case '2': {
                REQUIRE(e.tag == Y_EVENT_KEY_CHANGE_ADD);
                REQUIRE(!strcmp(e.key, "attr2"));
                REQUIRE(e.old_value == NULL);
                REQUIRE(!strcmp(youtput_read_string(e.new_value), "value2"));
                break;
            }
            default: FAIL("unrecognized case");
        }
    }

    // update attributes
    yxml_test_clean(t);
    txn = ytransaction_new(doc);
    yxmlelem_insert_attr(xml, txn, "attr1", "value11");
    yxmlelem_remove_attr(xml, txn, "attr2");
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->keys_len == 2);

    for (int i = 0; i < t->keys_len; i++) {
        YEventKeyChange e = t->keys[i];
        switch (e.key[4]) {
            case '1': {
                REQUIRE(e.tag == Y_EVENT_KEY_CHANGE_UPDATE);
                REQUIRE(!strcmp(e.key, "attr1"));
                REQUIRE(!strcmp(youtput_read_string(e.old_value), "value1"));
                REQUIRE(!strcmp(youtput_read_string(e.new_value), "value11"));
                break;
            }
            case '2': {
                REQUIRE(e.tag == Y_EVENT_KEY_CHANGE_DELETE);
                REQUIRE(!strcmp(e.key, "attr2"));
                REQUIRE(!strcmp(youtput_read_string(e.old_value), "value2"));
                REQUIRE(e.new_value == NULL);
                break;
            }
            default: FAIL("unrecognized case");
        }
    }

    // add children
    yxml_test_clean(t);
    txn = ytransaction_new(doc);
    YXmlElement* div = yxmlelem_insert_elem(xml, txn, 0, "div");
    YXmlElement* p = yxmlelem_insert_elem(xml, txn, 1, "p");
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 1);

    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[0].len == 2);

    yxmlelem_destroy(div);
    yxmlelem_destroy(p);

    // remove a child
    yxml_test_clean(t);
    txn = ytransaction_new(doc);
    yxmlelem_remove_range(xml, txn, 1, 1);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 2);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_RETAIN);
    REQUIRE(t->delta[0].len == 1);
    REQUIRE(t->delta[1].tag == Y_EVENT_CHANGE_DELETE);
    REQUIRE(t->delta[1].len == 1);

    // insert child again
    yxml_test_clean(t);
    txn = ytransaction_new(doc);
    div = yxmlelem_insert_elem(xml, txn, 1, "div");
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 2);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_RETAIN);
    REQUIRE(t->delta[0].len == 1);
    REQUIRE(t->delta[1].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[1].len == 1);

    yxmlelem_destroy(div);

    // free the observer and make sure that callback is no longer called
    yxml_test_clean(t);
    yxmlelem_unobserve(xml, sub);
    txn = ytransaction_new(doc);
    YXmlElement* inner = yxmlelem_insert_elem(xml, txn, 0, "head");
    ytransaction_commit(txn);

    REQUIRE(t->target == NULL);
    REQUIRE(t->delta == NULL);
    REQUIRE(t->keys == NULL);

    yxmlelem_destroy(inner);
    free(t);
    yxmlelem_destroy(xml);
    ydoc_destroy(doc);
}