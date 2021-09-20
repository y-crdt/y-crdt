#define DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN

#include <stdio.h>
#include <string.h>
#include "include/doctest.h"

extern "C" {
    #include "include/libyrs.h"
};

TEST_CASE("Update exchange basic") {
    // init
    YDoc* d1 = ydoc_new_with_id(1);
    YTransaction* t1 = ytxn_new(d1);
    YText* txt1 = ytxn_text(t1, "test");

    YDoc* d2 = ydoc_new_with_id(2);
    YTransaction* t2 = ytxn_new(d2);
    YText* txt2 = ytxn_text(t2, "test");

    // insert data at the same position on both peer texts
    ytext_insert(txt1, t1, 0, "world");
    ytext_insert(txt2, t2, 0, "hello ");

    // exchange updates
    int sv1_len = 0;
    unsigned char* sv1 = ytxn_state_vector_v1(t1, &sv1_len);

    int sv2_len = 0;
    unsigned char* sv2 = ytxn_state_vector_v1(t2, &sv2_len);

    int u1_len = 0;
    unsigned char* u1 = ytxn_state_diff_v1(t1, sv2, sv2_len, &u1_len);

    int u2_len = 0;
    unsigned char* u2 = ytxn_state_diff_v1(t2, sv1, sv1_len, &u2_len);

    ybinary_destroy(sv1, sv1_len);
    ybinary_destroy(sv2, sv2_len);

    // apply updates
    ytxn_apply(t1, u2, u2_len);
    ytxn_apply(t2, u1, u1_len);

    // make sure both peers produce the same output
    char* str1 = ytext_string(txt1, t1);
    char* str2 = ytext_string(txt2, t2);

    REQUIRE(!strcmp(str1, str2));

    ystr_destroy(str1);
    ystr_destroy(str2);

    ytext_destroy(txt2);
    ytxn_commit(t2);
    ydoc_destroy(d2);

    ytext_destroy(txt1);
    ytxn_commit(t1);
    ydoc_destroy(d1);
}

TEST_CASE("YText basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytxn_new(doc);
    YText* txt = ytxn_text(txn, "test");

    ytext_insert(txt, txn, 0, "hello");
    ytext_insert(txt, txn, 5, " world");
    ytext_remove_range(txt, txn, 0, 6);

    REQUIRE_EQ(ytext_len(txt), 5);

    char* str = ytext_string(txt, txn);
    REQUIRE(!strcmp(str, "world"));

    ystr_destroy(str);
    ytext_destroy(txt);
    ytxn_commit(txn);
    ydoc_destroy(doc);
}

TEST_CASE("YArray basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytxn_new(doc);
    YArray* arr = ytxn_array(txn, "test");

    YVal** nested = (YVal**)malloc(2 * sizeof(YVal**));
    nested[0] = yval_float(0.5);
    nested[1] = yval_bool(1);
    YVal* nested_array = yval_yarray(nested, 2);

    const int ARG_LEN = 3;

    YVal** args = (YVal**)malloc(ARG_LEN * sizeof(YVal**));
    args[0] = yval_str("hello");
    args[1] = yval_long(123);
    args[2] = nested_array;

    yarray_insert_range(arr, txn, 0, args, 2); //state after: ['hello', 123]
    yarray_insert_range(arr, txn, 0, (args + 2), 1); //state after: [ YArray([0.5, true]), 'hello', 123 ]
    yarray_remove_range(arr, txn, 1, 1); //state after: [ YArray([0.5, true]), 123 ]

    for (int j = 0; j < ARG_LEN; ++j) {
        yval_destroy(args[j]);
    }
    free(args);

    REQUIRE_EQ(yarray_len(arr), 2);

    YArrayIter* i = yarray_iter(arr, txn);

    // first outer YArray element should be another YArray([0.5, true])
    YVal* curr = yarray_iter_next(i);
    YArray* a = yval_read_yarray(curr);
    REQUIRE_EQ(yarray_len(a), 2);

    // read 0th element of inner YArray
    YVal* elem = yarray_get(a, txn, 0);
    REQUIRE_EQ(*yval_read_float(elem), 0.5);
    yval_destroy(elem);

    // read 1st element of inner YArray
    elem = yarray_get(a, txn, 1);
    REQUIRE_EQ(*yval_read_bool(elem), 1); // in C we use 1 to mark TRUE
    yval_destroy(elem);
    yval_destroy(curr);

    // second outer YArray element should be 123
    curr = yarray_iter_next(i);
    REQUIRE_EQ(*yval_read_long(curr), 123);
    yval_destroy(curr);

    curr = yarray_iter_next(i);
    REQUIRE(curr != NULL);

    yarray_iter_destroy(i);
    yarray_destroy(arr);
    ytxn_commit(txn);
    ydoc_destroy(doc);
}

TEST_CASE("YMap basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytxn_new(doc);
    YMap* map = ytxn_map(txn, "test");

    // insert 'a' => 'value'
    YVal* a = yval_str("value");

    ymap_insert(map, txn, "a", a);

    // insert 'b' -> [11,22]
    YVal** array = (YVal**) malloc(2 * sizeof(YVal**));
    array[0] = yval_long(11);
    array[1] = yval_long(22);
    YVal* b = yval_json_array(array, 2);

    ymap_insert(map, txn, "b", b);

    yval_destroy(a);
    yval_destroy(array[0]);
    yval_destroy(array[1]);
    free(array);

    REQUIRE_EQ(ymap_len(map, txn), 2);

    // iterate over entries
    YMapIter* i = ymap_iter(map, txn);

    YMapEntry* curr = ymap_iter_next(i);
    REQUIRE(!strcmp(curr->key, "a"));
    REQUIRE(!strcmp(yval_read_str(curr->value), "value"));
    ymap_entry_destroy(curr);

    curr = ymap_iter_next(i);
    REQUIRE(!strcmp(curr->key, "b"));
    array = yval_read_json_array(curr->value);
    REQUIRE_EQ(curr->value->len, 2);
    REQUIRE_EQ(*yval_read_long(array[0]), 11);
    REQUIRE_EQ(*yval_read_long(array[1]), 22);
    ymap_entry_destroy(curr);

    curr = ymap_iter_next(i);
    REQUIRE(curr == NULL);

    ymap_iter_destroy(i);

    // remove 'a' twice - second attempt should return null
    a = ymap_remove(map, txn, "a");
    REQUIRE(!strcmp(yval_read_str(a), "value"));
    yval_destroy(a);

    a = ymap_remove(map, txn, "a");
    REQUIRE(a == NULL);

    // get 'b' and read its contents
    b = ymap_get(map, txn, "b");
    array = yval_read_json_array(b);
    REQUIRE_EQ(b->len, 2);
    REQUIRE_EQ(*yval_read_long(array[0]), 11);
    REQUIRE_EQ(*yval_read_long(array[1]), 22);
    yval_destroy(b);

    // clear map
    ymap_remove_all(map, txn);
    REQUIRE_EQ(ymap_len(map, txn), 0);

    ymap_destroy(map);
    ytxn_commit(txn);
    ydoc_destroy(doc);
}

TEST_CASE("YXmlElement basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytxn_new(doc);
    YXmlElement* xml = ytxn_xml_elem(txn, "test");

    // XML attributes API
    yxml_insert_attr(xml, txn, "key1", "value1");
    yxml_insert_attr(xml, txn, "key2", "value2");

    REQUIRE_EQ(yxml_attr_len(xml, txn), 2);

    YXmlAttrIter* i = yxml_attr_iter(xml, txn);

    YXmlAttr* attr = yxml_attr_iter_next(i);
    REQUIRE(!strcmp(attr->name, "key1"));
    REQUIRE(!strcmp(attr->value, "value1"));
    yxml_attr_destroy(attr);

    attr = yxml_attr_iter_next(i);
    REQUIRE(!strcmp(attr->name, "key2"));
    REQUIRE(!strcmp(attr->value, "value2"));
    yxml_attr_destroy(attr);

    yxml_attr_iter_destroy(i);

    // XML children API
    YXmlElement* inner = yxml_insert_elem(xml, txn, 0, "p");
    YXmlText* inner_txt = yxml_insert_text(inner, txn, 0);
    yxmltext_insert(inner_txt, txn, 0, "hello");

    REQUIRE_EQ(yxml_child_len(xml, txn), 1);

    YXmlText* txt = yxml_insert_text(xml, txn, 1);
    yxmltext_insert(txt, txn, 0, "world");

    // check tag names
    char* tag = yxml_tag(inner);
    REQUIRE(!strcmp(tag, "p"));
    ystr_destroy(tag);

    tag = yxml_tag(xml);
    REQUIRE(!strcmp(tag, "UNDEFINED"));
    ystr_destroy(tag);

    // check parents
    YXmlElement* parent = yxml_parent(inner, txn);
    tag = yxml_tag(parent);
    REQUIRE(!strcmp(tag, "UNDEFINED"));
    ystr_destroy(tag);
    yxml_destroy(parent);

    parent = yxml_parent(xml, txn);
    REQUIRE(parent == NULL);
    yxml_destroy(parent);

    // check children traversal
    YVal* curr = yxml_first_child(xml, txn);
    YXmlElement* first = yval_read_yxml_elem(curr);
    REQUIRE(yxml_prev_sibling(first, txn) == NULL);
    char* str = yxml_string(first, txn);
    REQUIRE(!strcmp(str, "<p>hello</p>"));
    ystr_destroy(str);

    YVal* next = yxml_next_sibling(first, txn);
    yval_destroy(curr);
    YXmlText* second = yval_read_yxmltext(next);
    REQUIRE(yxmltext_next_sibling(second, txn) == NULL);
    str = yxmltext_string(second, txn);
    REQUIRE(!(strcmp(str, "world")));
    ystr_destroy(str);
    yval_destroy(curr);

    // check tree walker - expected order:
    // - UNDEFINED // (XML root element)
    // - p
    // - hello
    // - world
    YXmlTreeWalker* w = yxml_tree_walker(xml, txn);

    curr = yxml_tree_walker_next(w);
    YXmlElement* e = yval_read_yxml_elem(curr);
    str = yxml_string(e, txn);
    REQUIRE(!strcmp(str, "<UNDEFINED><p>hello</p>world</UNDEFINED>"));
    ystr_destroy(str);
    yval_destroy(curr);

    curr = yxml_tree_walker_next(w);
    e = yval_read_yxml_elem(curr);
    str = yxml_string(e, txn);
    REQUIRE(!strcmp(str, "<p>hello</p>"));
    ystr_destroy(str);
    yval_destroy(curr);

    curr = yxml_tree_walker_next(w);
    YXmlText* t = yval_read_yxmltext(curr);
    str = yxmltext_string(t, txn);
    REQUIRE(!strcmp(str, "hello"));
    ystr_destroy(str);
    yval_destroy(curr);

    curr = yxml_tree_walker_next(w);
    t = yval_read_yxmltext(curr);
    str = yxmltext_string(t, txn);
    REQUIRE(!strcmp(str, "world"));
    ystr_destroy(str);
    yval_destroy(curr);

    curr = yxml_tree_walker_next(w);
    REQUIRE(curr == NULL);

    yxml_tree_walker_destroy(w);

    yxmltext_destroy(inner_txt);
    yxmltext_destroy(txt);
    yxml_destroy(inner);

    yxml_destroy(xml);
    ytxn_commit(txn);
    ydoc_destroy(doc);
}