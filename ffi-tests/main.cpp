#define DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN

#include <stdio.h>
#include <string.h>
#include "include/doctest.h"

extern "C" {
    #include "include/libyrs.h"
};

TEST_CASE("YText basic") {
    YDoc* doc = ydoc_new_with_id(1);
    YTransaction* txn = ytxn_new(doc);
    YText* txt = ytxn_text(txn, "test");

    ytext_insert(txt, txn, 0, "hello");
    ytext_insert(txt, txn, 5, " world");
    ytext_remove_range(txt, txn, 0, 6);

    REQUIRE_EQ(ytext_len(txt), 5);

    char* str = ytext_string(txt, txn);
    REQUIRE(strcmp(str, "world"));

    ystr_destroy(str);
    ytext_destroy(txt);
    ytxn_commit(txn);
    ydoc_destroy(doc);
}