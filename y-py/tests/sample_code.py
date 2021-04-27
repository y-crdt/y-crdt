import y_py

# shows all the functions available in module y_py
print(dir(y_py))


# I used the following JS code to generate a binary buffer :

# const ydoc1 = new Y.Doc()
# ydoc1.getArray('array_doc1').insert(0, ['example 2, array doc 1, 0th value'])
# ydoc1.getArray('array_doc1').insert(1, ['example 2, array doc 1, 0th value'])
# ydoc1.getArray('array2_doc1').insert(0, ['example 2, array 2 doc 1, 0th value'])
# let currentState1 = Y.encodeStateAsUpdate(ydoc1)

bin_buff1 = [
    1, 2, 242, 143, 191, 196, 7, 0, 8, 1, 10, 97, 114, 114, 97, 121, 95, 100, 111,
    99, 49, 2, 119, 33, 101, 120, 97, 109, 112, 108, 101, 32, 50, 44, 32, 97, 114, 114,
    97, 121, 32, 100, 111, 99, 32, 49, 44, 32, 48, 116, 104, 32, 118, 97, 108, 117, 101,
    119, 33, 101, 120, 97, 109, 112, 108, 101, 32, 50, 44, 32, 97, 114, 114, 97, 121, 32,
    100, 111, 99, 32, 49, 44, 32, 48, 116, 104, 32, 118, 97, 108, 117, 101, 8, 1, 11, 97,
    114, 114, 97, 121, 50, 95, 100, 111, 99, 49, 1, 119, 35, 101, 120, 97, 109, 112, 108,
    101, 32, 50, 44, 32, 97, 114, 114, 97, 121, 32, 50, 32, 100, 111, 99, 32, 49, 44, 32,
    48, 116, 104, 32, 118, 97, 108, 117, 101, 0,
]


result = y_py.encode_state_vector_from_update(bin_buff1)

print(result)

# test here the other functions :
# y_py.merge_updates
# y_py.diff_updates
