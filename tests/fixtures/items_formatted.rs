//! Changed file docs.
const SCALE : i32 = 2 ; // ordinary comment
static OFFSET : i32 = 3 ;
const OTHER : i32 = 10 ;
#[doc = "Changed helper docs"]
fn helper ( ) -> i32 {
    leaf ( ) + SCALE
}
fn leaf ( ) -> i32 { 7 }
#[doc = "Changed operation docs"]
fn selected ( ) -> i32 {
    helper ( ) + OFFSET // comment
}
fn separate ( ) -> i32 { OTHER }
fn ops ( id : i32 , enabled : bool ) -> i32 {
    match id {
        1000 | 1100 => helper ( ) ,
        1200 if enabled => SCALE + OFFSET ,
        9 => 900 ,
        _ => 0 ,
    }
}
fn text ( value : & str ) -> i32 {
    match value {
        "hello" | "world" => helper ( ) ,
        "a:b=c" => 2 ,
        _ => 0 ,
    }
}
mod nested {
    #[doc = "Different module function docs"]
    pub fn selected ( ) -> i32 { 42 }
    mod deeper {
        fn nested_only ( ) -> i32 { 8 }
    }
}
struct Worker ;
impl Worker {
    fn selected ( & self ) -> i32 { 3 }
    fn method_only ( & self ) -> i32 { 4 }
}
