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
    Branch* txt1 = ytext(t1, "test");

    YDoc* d2 = ydoc_new_with_id(2);
    YTransaction* t2 = ytransaction_new(d2);
    Branch* txt2 = ytext(t2, "test");

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
    char* str1 = ytext_string(txt1);
    char* str2 = ytext_string(txt2);

    REQUIRE(!strcmp(str1, str2));

    ystring_destroy(str1);
    ystring_destroy(str2);

    ytransaction_commit(t2);
    ydoc_destroy(d2);

    ytransaction_commit(t1);
    ydoc_destroy(d1);
}

TEST_CASE("YText basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* txt = ytext(txn, "test");

    ytext_insert(txt, txn, 0, "hello", NULL);
    ytext_insert(txt, txn, 5, " world", NULL);
    ytext_remove_range(txt, txn, 0, 6);

    REQUIRE_EQ(ytext_len(txt), 5);

    char* str = ytext_string(txt);
    REQUIRE(!strcmp(str, "world"));

    ystring_destroy(str);
    ytransaction_commit(txn);
    ydoc_destroy(doc);
}

TEST_CASE("YArray basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* arr = yarray(txn, "test");

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

    YArrayIter* i = yarray_iter(arr);

    // first outer YArray element should be another YArray([0.5, true])
    YOutput* curr = yarray_iter_next(i);
    Branch* a = youtput_read_yarray(curr);
    REQUIRE_EQ(yarray_len(a), 2);

    // read 0th element of inner YArray
    YOutput* elem = yarray_get(a, 0);
    REQUIRE_EQ(*youtput_read_float(elem), 0.5);
    youtput_destroy(elem);

    // read 1st element of inner YArray
    elem = yarray_get(a, 1);
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
    ytransaction_commit(txn);
    ydoc_destroy(doc);
}

TEST_CASE("YMap basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* map = ymap(txn, "test");

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

    REQUIRE_EQ(ymap_len(map), 2);

    // iterate over entries
    YMapIter* i = ymap_iter(map);
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
    YOutput* out = ymap_get(map, "b");
    YOutput* output = youtput_read_json_array(out);
    REQUIRE_EQ(out->len, 2);
    REQUIRE_EQ(*youtput_read_long(&output[0]), 11);
    REQUIRE_EQ(*youtput_read_long(&output[1]), 22);
    youtput_destroy(out);

    // clear map
    ymap_remove_all(map, txn);
    REQUIRE_EQ(ymap_len(map), 0);

    ytransaction_commit(txn);
    ydoc_destroy(doc);
}

TEST_CASE("YXmlElement basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* xml = yxmlelem(txn, "test");

    // XML attributes API
    yxmlelem_insert_attr(xml, txn, "key1", "value1");
    yxmlelem_insert_attr(xml, txn, "key2", "value2");

    YXmlAttrIter* i = yxmlelem_attr_iter(xml);
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
    Branch* inner = yxmlelem_insert_elem(xml, txn, 0, "p");
    Branch* inner_txt = yxmlelem_insert_text(inner, txn, 0);
    yxmltext_insert(inner_txt, txn, 0, "hello", NULL);

    REQUIRE_EQ(yxmlelem_child_len(xml), 1);

    Branch* txt = yxmlelem_insert_text(xml, txn, 1);
    yxmltext_insert(txt, txn, 0, "world", NULL);

    // check tag names
    char* tag = yxmlelem_tag(inner);
    REQUIRE(!strcmp(tag, "p"));
    ystring_destroy(tag);

    tag = yxmlelem_tag(xml);
    REQUIRE(!strcmp(tag, "UNDEFINED"));
    ystring_destroy(tag);

    // check parents
    Branch* parent = yxmlelem_parent(inner);
    tag = yxmlelem_tag(parent);
    REQUIRE(!strcmp(tag, "UNDEFINED"));
    ystring_destroy(tag);

    parent = yxmlelem_parent(xml);
    REQUIRE(parent == NULL);

    // check children traversal
    YOutput* curr = yxmlelem_first_child(xml);
    Branch* first = youtput_read_yxmlelem(curr);
    REQUIRE(yxmlelem_prev_sibling(first) == NULL);
    char* str = yxmlelem_string(first);
    REQUIRE(!strcmp(str, "<p>hello</p>"));
    ystring_destroy(str);

    YOutput* next = yxmlelem_next_sibling(first);
    youtput_destroy(curr);
    Branch* second = youtput_read_yxmltext(next);
    REQUIRE(yxmltext_next_sibling(second) == NULL);
    str = yxmltext_string(second);
    REQUIRE(!(strcmp(str, "world")));
    ystring_destroy(str);

    // check tree walker - expected order:
    // - p
    // - hello
    // - world
    YXmlTreeWalker* w = yxmlelem_tree_walker(xml);
    Branch* e;

    curr = yxmlelem_tree_walker_next(w);
    e = youtput_read_yxmlelem(curr);
    str = yxmlelem_string(e);
    REQUIRE(!strcmp(str, "<p>hello</p>"));
    ystring_destroy(str);
    youtput_destroy(curr);

    curr = yxmlelem_tree_walker_next(w);
    Branch* t = youtput_read_yxmltext(curr);
    str = yxmltext_string(t);
    REQUIRE(!strcmp(str, "hello"));
    ystring_destroy(str);
    youtput_destroy(curr);

    curr = yxmlelem_tree_walker_next(w);
    t = youtput_read_yxmltext(curr);
    str = yxmltext_string(t);
    REQUIRE(!strcmp(str, "world"));
    ystring_destroy(str);
    youtput_destroy(curr);

    curr = yxmlelem_tree_walker_next(w);
    REQUIRE(curr == NULL);

    yxmlelem_tree_walker_destroy(w);

    ytransaction_commit(txn);
    ydoc_destroy(doc);
}

typedef struct YTextEventTest {
    int delta_len;
    YDelta* delta;
    Branch* target;
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
    ytext_delta_destroy(t->delta, t->delta_len);
    t->target = NULL;
    t->delta = NULL;
}

TEST_CASE("YText observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* txt = ytext(txn, "test");

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
    ydoc_destroy(doc);
}

TEST_CASE("YText insert embed") {
    YDoc *doc = ydoc_new_with_id(1);
    YTransaction *txn = ytransaction_new(doc);
    Branch* txt = ytext(txn, "test");

    YTextEventTest *t = ytext_event_test_new();
    unsigned int sub = ytext_observe(txt, (void *) t, &ytext_test_observe);

    char* _bold = (char*)"bold";
    YInput _true = yinput_bool(1);
    YInput attrs1 = yinput_json_map(&_bold, &_true, 1);

    char* _width = (char*)"width";
    YInput _100 = yinput_long(100);
    YInput attrs2 = yinput_json_map(&_width, &_100, 1);

    char* _image = (char*)"image";
    YInput _image_src = yinput_string("imageSrc.png");
    YInput embed = yinput_json_map(&_image, &_image_src, 1);

    ytext_insert(txt, txn, 0, "ab", &attrs1);
    ytext_insert_embed(txt, txn, 1, &embed, &attrs2);
    ytransaction_commit(txn);

    REQUIRE(t->delta_len == 3);

    YDelta d = t->delta[0];
    REQUIRE(d.tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(d.insert->len == 1);
    REQUIRE(strcmp(youtput_read_string(d.insert), "a") == 0);
    REQUIRE(d.attributes_len == 1);
    REQUIRE(strcmp(d.attributes->key, "bold") == 0);
    REQUIRE(*youtput_read_bool(&d.attributes->value) == 1);

    d = t->delta[1];
    REQUIRE(d.tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(d.len == 1);
    REQUIRE(d.attributes_len == 1);
    REQUIRE(strcmp(d.attributes->key, "width") == 0);
    REQUIRE(*youtput_read_long(&d.attributes->value) == 100);
    YMapEntry* e = youtput_read_json_map(d.insert);
    REQUIRE(d.insert->len == 1);
    REQUIRE(strcmp(e->key, "image") == 0);
    REQUIRE(strcmp(youtput_read_string(&e->value), "imageSrc.png") == 0);

    d = t->delta[2];
    REQUIRE(d.tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(d.insert->len == 1);
    REQUIRE(strcmp(youtput_read_string(d.insert), "b") == 0);
    REQUIRE(d.attributes_len == 1);
    REQUIRE(strcmp(d.attributes->key, "bold") == 0);
    REQUIRE(*youtput_read_bool(&d.attributes->value) == 1);

    ytext_test_clean(t);
    ytext_unobserve(txt, sub);
    free(t);
    ydoc_destroy(doc);
}

typedef struct YArrayEventTest {
    int delta_len;
    YEventChange* delta;
    Branch* target;
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
    yevent_delta_destroy(t->delta, t->delta_len);
    t->target = NULL;
    t->delta = NULL;
}

TEST_CASE("YArray observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* array = yarray(txn, "test");

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
    ydoc_destroy(doc);
}

typedef struct YMapEventTest {
    int keys_len;
    YEventKeyChange * keys;
    Branch* target;
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
    yevent_keys_destroy(t->keys, t->keys_len);
    t->target = NULL;
    t->keys = NULL;
}

TEST_CASE("YMap observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* map = ymap(txn, "test");

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

    ydoc_destroy(doc);
}

typedef struct YXmlTextEventTest {
    int delta_len;
    int keys_len;
    YDelta* delta;
    Branch* target;
    YEventKeyChange *keys;
} YXmlTextEventTest;

YXmlTextEventTest* yxmltext_event_test_new() {
    YXmlTextEventTest* t = (YXmlTextEventTest*)malloc(sizeof(YXmlTextEventTest));
    t->target = NULL;
    t->delta = NULL;
    t->delta_len = 0;
    t->keys = NULL;
    t->keys_len = 0;

    return t;
}

void yxmltext_test_observe(void* state, const YXmlTextEvent* e) {
    YXmlTextEventTest* t = (YXmlTextEventTest*) state;
    t->target = yxmltext_event_target(e);
    t->delta = yxmltext_event_delta(e, &t->delta_len);
    t->keys = yxmltext_event_keys(e, &t->keys_len);
}

void yxmltext_test_clean(YXmlTextEventTest* t) {
    ytext_delta_destroy(t->delta, t->delta_len);
    yevent_keys_destroy(t->keys, t->keys_len);
    t->target = NULL;
    t->delta = NULL;
    t->keys = NULL;
}

TEST_CASE("YXmlText observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* txt = yxmltext(txn, "test");

    YXmlTextEventTest* t = yxmltext_event_test_new();
    unsigned int sub = yxmltext_observe(txt, (void*)t, &yxmltext_test_observe);

    // insert initial data to an empty YText
    yxmltext_insert(txt, txn, 0, "abcd", NULL);
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 1);
    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[0].insert->len == 4);

    // remove 2 chars from the middle
    yxmltext_test_clean(t);
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
    yxmltext_test_clean(t);
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
    yxmltext_test_clean(t);
    yxmltext_unobserve(txt, sub);

    txn = ytransaction_new(doc);
    yxmltext_insert(txt, txn, 1, "fgh", NULL);
    ytransaction_commit(txn);

    REQUIRE(t->target == NULL);
    REQUIRE(t->delta == NULL);

    free(t);
    ydoc_destroy(doc);
}

typedef struct YXmlEventTest {
    Branch* target;
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
    t->target = yxmlelem_event_target(e);
    t->keys = yxmlelem_event_keys(e, &t->keys_len);
    t->delta = yxmlelem_event_delta(e, &t->delta_len);
}

void yxml_test_clean(YXmlEventTest* t) {
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
    Branch* xml = yxmlelem(txn, "test");

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
    Branch* div = yxmlelem_insert_elem(xml, txn, 0, "div");
    Branch* p = yxmlelem_insert_elem(xml, txn, 1, "p");
    ytransaction_commit(txn);

    REQUIRE(t->target != NULL);
    REQUIRE(t->delta_len == 1);

    REQUIRE(t->delta[0].tag == Y_EVENT_CHANGE_ADD);
    REQUIRE(t->delta[0].len == 2);

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


    // free the observer and make sure that callback is no longer called
    yxml_test_clean(t);
    yxmlelem_unobserve(xml, sub);
    txn = ytransaction_new(doc);
    Branch* inner = yxmlelem_insert_elem(xml, txn, 0, "head");
    ytransaction_commit(txn);

    REQUIRE(t->target == NULL);
    REQUIRE(t->delta == NULL);
    REQUIRE(t->keys == NULL);

    free(t);
    ydoc_destroy(doc);
}

typedef struct YDeepObserveTest {
    YPathSegment* path[4];
    int path_lens[4];
    int count;
} YDeepObserveTest;

YDeepObserveTest* new_ydeepobserve_test() {
    YDeepObserveTest* e = (YDeepObserveTest*)malloc(sizeof(YDeepObserveTest));
    memset(e->path, 0, sizeof(YPathSegment*) * 4);
    memset(e->path_lens, 0, sizeof(int) * 4);
    e->count = 0;
    return e;
}

void ydeepobserve_test_clean(YDeepObserveTest* test) {
    for (int i = 0; i < test->count; i++) {
        ypath_destroy(test->path[i], test->path_lens[i]);
    }
    test->count = 0;
}

void ydeepobserve_test(void* state, int event_count, const YEvent* events) {
    YDeepObserveTest* test = (YDeepObserveTest*)state;
    // cleanup previous state
    ydeepobserve_test_clean(test);

    for (int i = 0; i < event_count; i++) {
        YEvent e = events[i];
        int path_len = 0;
        switch (e.tag) {
            case Y_ARRAY: {
                YArrayEvent nested = e.content.array;
                test->path[i] = yarray_event_path(&nested, &path_len);
                test->path_lens[i] = path_len;
                test->count++;
                break;
            }
            case Y_MAP: {
                YMapEvent nested = e.content.map;
                test->path[i] = ymap_event_path(&nested, &path_len);
                test->path_lens[i] = path_len;
                test->count++;
                break;
            }
            // we don't use other Y types in this test
        }
    }
}

TEST_CASE("YArray deep observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* array = yarray(txn, "test");
    ytransaction_commit(txn);

    YDeepObserveTest* state = new_ydeepobserve_test();
    unsigned int subscription_id = yobserve_deep(array, (void *) state, ydeepobserve_test);

    txn = ytransaction_new(doc);
    YInput input = yinput_ymap(NULL, NULL, 0);
    yarray_insert_range(array, txn, 0, &input, 1);
    ytransaction_commit(txn);

    txn = ytransaction_new(doc);
    YOutput* output = yarray_get(array, 0);
    Branch* map = youtput_read_ymap(output);
    input = yinput_string("value");
    ymap_insert(map, txn, "key", &input);
    input = yinput_long(0);
    yarray_insert_range(array, txn, 0, &input, 1);
    ytransaction_commit(txn);

    REQUIRE(state->count == 2);
    int path_len = state->path_lens[0];
    YPathSegment* path = state->path[0];
    REQUIRE(path_len == 0);
    path_len = state->path_lens[1];
    path = state->path[1];
    REQUIRE(path_len == 1);
    REQUIRE(path[0].tag == Y_EVENT_PATH_INDEX);
    REQUIRE(path[0].value.index == 1);

    yunobserve_deep(array, subscription_id);
    ydeepobserve_test_clean(state);
    free(state);
    ydoc_destroy(doc);
}

TEST_CASE("YMap deep observe") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytransaction_new(doc);
    Branch* map = ymap(txn, "test");
    ytransaction_commit(txn);

    YDeepObserveTest* state = new_ydeepobserve_test();
    unsigned int subscription_id = yobserve_deep(map, (void *) state, ydeepobserve_test);

    /* map.set(txn, 'map', new Y.YMap()) */
    txn = ytransaction_new(doc);
    YInput input = yinput_ymap(NULL, NULL, 0);
    ymap_insert(map, txn, "map", &input);
    ytransaction_commit(txn);

    // path: []
    REQUIRE(state->count == 1);
    int path_len = state->path_lens[0];
    YPathSegment* path = state->path[0];
    REQUIRE(path_len == 0);

    /* map.get('map').set(txn, 'array', new Y.YArray()) */
    txn = ytransaction_new(doc);
    YOutput* output = ymap_get(map, "map");
    Branch* nested = youtput_read_ymap(output);
    input = yinput_yarray(NULL, 0);
    ymap_insert(nested, txn, "array", &input);
    ytransaction_commit(txn);

    // path: ['map']
    REQUIRE(state->count == 1);
    path_len = state->path_lens[0];
    path = state->path[0];
    REQUIRE(path_len == 1);
    REQUIRE(path[0].tag == Y_EVENT_PATH_KEY);
    REQUIRE(!strcmp(path[0].value.key, "map"));

    /* map.get('map').get('array').insert(txn, 0, ['content']) */
    txn = ytransaction_new(doc);
    output = ymap_get(map, "map");
    nested = youtput_read_ymap(output);
    output = ymap_get(nested, "array");
    nested = youtput_read_yarray(output);
    input = yinput_string("content");
    yarray_insert_range(nested, txn, 0, &input, 1);
    ytransaction_commit(txn);

    // path: ['map', 'array']
    REQUIRE(state->count == 1);
    path_len = state->path_lens[0];
    path = state->path[0];
    REQUIRE(path_len == 2);
    REQUIRE(path[0].tag == Y_EVENT_PATH_KEY);
    REQUIRE(!strcmp(path[0].value.key, "map"));
    REQUIRE(path[1].tag == Y_EVENT_PATH_KEY);
    REQUIRE(!strcmp(path[1].value.key, "array"));

    yunobserve_deep(map, subscription_id);
    ydeepobserve_test_clean(state);
    free(state);
    ydoc_destroy(doc);
}

typedef struct {
    int len;
    unsigned char* update;
    int incoming_len;
    unsigned char* incoming_update;
} ObserveUpdatesTest;

void reset_observe_updates(ObserveUpdatesTest* t) {
    if (NULL != t->incoming_update) {
        free(t->incoming_update);
        t->incoming_update = NULL;
        t->incoming_len = 0;
    }
    if (NULL != t->update) {
        //TODO: since on Windows Rust uses HeapAlloc/HeapFree - the code below should take that into account
        free(t->update);
        t->update = NULL;
        t->len = 0;
    }
}

void observe_updates(void* state, int len, const unsigned char* bytes) {
    ObserveUpdatesTest* t = (ObserveUpdatesTest*)state;
    t->incoming_len = len;
    void* buf = malloc(sizeof(unsigned char*) * len);
    memcpy(buf, (void*)bytes, (size_t)len);
    t->incoming_update = (unsigned char*)buf;
}

TEST_CASE("YDoc observe updates V1") {
    YDoc *doc1 = ydoc_new_with_id(1);
    YTransaction *txn = ytransaction_new(doc1);
    Branch *txt1 = ytext(txn, "test");
    ytext_insert(txt1, txn, 0, "hello", NULL);
    ObserveUpdatesTest* t = (ObserveUpdatesTest*)malloc(sizeof(ObserveUpdatesTest));
    t->incoming_len = 0;
    t->incoming_update = NULL;
    t->update = ytransaction_state_diff_v1(txn, NULL, 0, &t->len);
    ytransaction_commit(txn);

    YDoc *doc2 = ydoc_new_with_id(2);
    txn = ytransaction_new(doc2);
    Branch *txt2 = ytext(txn, "test");
    unsigned int subscription_id = ydoc_observe_updates_v1(doc2, t, observe_updates);
    ytransaction_apply(txn, t->update, t->len);
    ytransaction_commit(txn);

    REQUIRE_EQ(t->len, t->incoming_len);
    REQUIRE(0 == memcmp((void*)t->update, (void*)t->incoming_update, (size_t)t->len));
    reset_observe_updates(t);

    // check unsubscribe
    ydoc_unobserve_updates_v1(doc2, subscription_id);

    txn = ytransaction_new(doc1);
    ytext_insert(txt1, txn, 5, " world", NULL);
    t->update = ytransaction_state_diff_v1(txn, NULL, 0, &t->len);
    ytransaction_commit(txn);

    txn = ytransaction_new(doc2);
    ytransaction_apply(txn, t->update, t->len);
    ytransaction_commit(txn);

    REQUIRE_EQ(t->incoming_len, 0);
    REQUIRE(t->incoming_update == NULL);

    ydoc_destroy(doc1);
    ydoc_destroy(doc2);
    free(t);
}

TEST_CASE("YDoc observe updates V2") {
    YDoc *doc1 = ydoc_new_with_id(1);
    YTransaction *txn = ytransaction_new(doc1);
    Branch *txt1 = ytext(txn, "test");
    ytext_insert(txt1, txn, 0, "hello", NULL);
    ObserveUpdatesTest* t = (ObserveUpdatesTest*)malloc(sizeof(ObserveUpdatesTest));
    t->incoming_len = 0;
    t->incoming_update = NULL;
    t->update = ytransaction_state_diff_v2(txn, NULL, 0, &t->len);
    ytransaction_commit(txn);

    YDoc *doc2 = ydoc_new_with_id(2);
    txn = ytransaction_new(doc2);
    Branch *txt2 = ytext(txn, "test");
    unsigned int subscription_id = ydoc_observe_updates_v2(doc2, t, observe_updates);
    ytransaction_apply_v2(txn, t->update, t->len);
    ytransaction_commit(txn);

    REQUIRE_EQ(t->len, t->incoming_len);
    REQUIRE(0 == memcmp((void*)t->update, (void*)t->incoming_update, (size_t)t->len));
    reset_observe_updates(t);

    // check unsubscribe
    ydoc_unobserve_updates_v2(doc2, subscription_id);

    txn = ytransaction_new(doc1);
    ytext_insert(txt1, txn, 5, " world", NULL);
    t->update = ytransaction_state_diff_v2(txn, NULL, 0, &t->len);
    ytransaction_commit(txn);

    txn = ytransaction_new(doc2);
    ytransaction_apply_v2(txn, t->update, t->len);
    ytransaction_commit(txn);

    REQUIRE_EQ(t->incoming_len, 0);
    REQUIRE(t->incoming_update == NULL);

    ydoc_destroy(doc1);
    ydoc_destroy(doc2);
    free(t);
}


int ystate_vector_eq(YStateVector* a, YStateVector* b) {
    if (a->entries_count != b->entries_count)
        return 0;

    for (int i = 0; i < a->entries_count; i++) {
        long ida = a->client_ids[i];
        long idb = b->client_ids[i];
        if (ida != idb)
            return 0;

        int clocka = a->clocks[i];
        int clockb = b->clocks[i];
        if (clocka != clockb)
            return 0;
    }

    return 1;
}

int ydelete_set_eq(YDeleteSet* a, YDeleteSet* b) {
    if (a->entries_count != b->entries_count)
        return 0;

    for (int i = 0; i < a->entries_count; i++) {
        long ida = a->client_ids[i];
        long idb = b->client_ids[i];
        if (ida != idb)
            return 0;

        YIdRangeSeq seqa = a->ranges[i];
        YIdRangeSeq seqb = b->ranges[i];

        if (seqa.len != seqb.len)
            return 0;

        for (int j = 0; j < seqa.len; j++) {
            YIdRange ra = seqa.seq[j];
            YIdRange rb = seqb.seq[j];

            if (ra.start != rb.start || ra.end != rb.end)
                return 0;
        }
    }

    return 1;
}

typedef struct {
    int calls;
    YStateVector before_state;
    YStateVector after_state;
    YDeleteSet delete_set;
} AfterTransactionTest;

void observe_after_transaction(void* state, YAfterTransactionEvent* e) {
    AfterTransactionTest* t = (AfterTransactionTest*)state;
    t->calls++;
    REQUIRE(ystate_vector_eq(&t->before_state, &e->before_state));
    REQUIRE(ystate_vector_eq(&t->after_state, &e->after_state));
    REQUIRE(ydelete_set_eq(&t->delete_set, &e->delete_set));
}

TEST_CASE("YDoc observe after transaction") {
    long long CLIENT_ID = 1;
    AfterTransactionTest t;
    t.calls = 0;
    t.delete_set.entries_count = 0;
    t.before_state.entries_count = 0;

    t.after_state.entries_count = 1;
    t.after_state.client_ids = &CLIENT_ID;
    int CLOCK = 11;
    t.after_state.clocks = &CLOCK;

    YDoc *doc1 = ydoc_new_with_id(CLIENT_ID);
    unsigned int subscription_id = ydoc_observe_after_transaction(doc1, &t, observe_after_transaction);

    YTransaction *txn = ytransaction_new(doc1);
    Branch *txt1 = ytext(txn, "test");
    ytext_insert(txt1, txn, 0, "hello world", NULL);
    ytransaction_commit(txn);

    REQUIRE_EQ(t.calls, 1);

    // on the second transaction, before state will be equal after state from previous transaction
    t.before_state.entries_count = t.after_state.entries_count;
    t.before_state.client_ids = t.after_state.client_ids;
    t.before_state.clocks = t.after_state.clocks;

    t.delete_set.entries_count = 1;
    t.delete_set.client_ids = &CLIENT_ID;
    t.delete_set.ranges = (YIdRangeSeq*)malloc(sizeof(YIdRangeSeq*) * 1);
    t.delete_set.ranges[0].len = 1;
    YIdRange range;
    range.start = 2;
    range.end = 9;
    t.delete_set.ranges[0].seq = &range;

    txn = ytransaction_new(doc1);
    ytext_remove_range(txt1, txn, 2, 7);
    ytransaction_commit(txn);

    REQUIRE_EQ(t.calls, 2);

    ydoc_unobserve_after_transaction(doc1, subscription_id);

    txn = ytransaction_new(doc1);
    ytext_insert(txt1, txn, 4, " the door", NULL);
    ytransaction_commit(txn);

    REQUIRE_EQ(t.calls, 2); // number of calls should remain unchanged

    ydoc_destroy(doc1);
}

TEST_CASE("YDoc snapshots") {
    YOptions o;
    o.encoding = Y_OFFSET_UTF16;
    o.id = 1;
    o.skip_gc = 1;

    YDoc* doc = ydoc_new_with_options(o);
    YTransaction* txn = ytransaction_new(doc);
    Branch* txt = ytext(txn, "test");

    ytext_insert(txt, txn, 0, "hello", NULL);

    int snapshot_len = 0;
    unsigned char* snapshot = ytransaction_snapshot(txn, &snapshot_len);

    ytext_insert(txt, txn, 5, " world", NULL);

    int update_len = 0;
    unsigned char* update = ytransaction_encode_state_from_snapshot_v1(txn, snapshot, snapshot_len, &update_len);

    ytransaction_commit(txn);
    ydoc_destroy(doc);

    doc = ydoc_new_with_id(1);
    txn = ytransaction_new(doc);
    txt = ytext(txn, "test");

    ytransaction_apply(txn, update, update_len);

    char* str = ytext_string(txt);
    printf("result: %s", str);
    REQUIRE(!strcmp(str, "hello"));

    ystring_destroy(str);
    ytransaction_commit(txn);
    ydoc_destroy(doc);
}