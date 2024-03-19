//# publish --print-bytecode
module 0xcafe::vectors {
    use std::vector;

    fun make_big_vector(a: u64): vector<u64> {
        let x = vector[a,
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            {a = a + 1; a},
            ];
        let y = x;
        y
    }

    fun sum(a: &vector<u64>): u64 {
        let sum = 0;
        vector::for_each_ref(a, |elt| { sum = sum + *elt});
        sum
    }

    public fun test_big_vector() {
        let v = make_big_vector(0);
        let len = vector::length(&v);
        let sum = sum(&v);
        assert!(len == 75, len);
        assert!(sum == (len * (len -1)) / 2, sum);
    }
}

//# run 0xcafe::vectors::test_big_vector
