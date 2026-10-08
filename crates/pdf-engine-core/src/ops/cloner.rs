//! Deep object graph cloner for transferring objects and pages across documents.
//!
//! Preserves object graph topology, prevents cycles, deduplicates shared resources
//! (fonts, images, color spaces), and re-parents pages to the destination document.

use crate::cos::{ObjectId, PdfDictionary, PdfDocument, PdfObject, PdfStream};
use crate::error::{PdfError, PdfResult};
use std::collections::{HashMap, HashSet};

/// Manages transitive object graph cloning between a source and destination `PdfDocument`.
pub struct ObjectCloner {
    /// Maps source `ObjectId` to newly allocated destination `ObjectId`.
    pub id_map: HashMap<ObjectId, ObjectId>,
    /// Tracks active stack of object IDs to detect circular references.
    active_stack: HashSet<ObjectId>,
    /// Source document catalog ID (excluded from cloning).
    src_catalog_id: Option<ObjectId>,
    /// Source document pages root ID (excluded from cloning).
    src_pages_id: Option<ObjectId>,
}

impl ObjectCloner {
    /// Creates a new `ObjectCloner` tailored for the given source document.
    pub fn new(src_doc: &mut PdfDocument) -> PdfResult<Self> {
        let src_catalog_id = src_doc.catalog_id();
        let src_pages_id = src_doc.pages_id().ok();
        Ok(Self {
            id_map: HashMap::new(),
            active_stack: HashSet::new(),
            src_catalog_id,
            src_pages_id,
        })
    }

    /// Clones a page from `src_doc` into `dest_doc`, re-parenting it to `dest_pages_id`.
    /// Appends the new page to `dest_doc`'s `/Pages` node and increments `/Count`.
    pub fn import_page(
        &mut self,
        src_doc: &mut PdfDocument,
        src_page_id: ObjectId,
        dest_doc: &mut PdfDocument,
    ) -> PdfResult<ObjectId> {
        let dest_pages_id = dest_doc.pages_id()?;

        let src_page_obj = src_doc.get_object(src_page_id)?;
        let page_dict = match src_page_obj {
            PdfObject::Dictionary(d) => d,
            _ => {
                return Err(PdfError::TypeMismatch {
                    id: src_page_id.number,
                    gen: src_page_id.generation,
                    expected: "Dictionary",
                    found: "Non-dictionary page",
                })
            }
        };

        // Allocate ID for the cloned page in dest_doc and reserve immediately in objects map
        let dest_page_id = dest_doc.alloc_object_id();
        dest_doc.set_object(dest_page_id, PdfObject::Null);
        self.id_map.insert(src_page_id, dest_page_id);

        // Deep clone all entries in the page dictionary, ensuring /Parent points to dest_pages_id
        let mut cloned_dict = PdfDictionary::new();
        for (key, val) in page_dict.0 {
            if key.as_str() == "Parent" {
                cloned_dict.insert(key, dest_pages_id);
            } else {
                let cloned_val = self.clone_object(&val, src_doc, dest_doc)?;
                cloned_dict.insert(key, cloned_val);
            }
        }
        cloned_dict.insert("Parent", dest_pages_id);

        // Store cloned page in destination
        dest_doc.set_object(dest_page_id, PdfObject::Dictionary(cloned_dict));

        // Update destination Pages tree (/Kids and /Count)
        let mut dest_pages_dict = match dest_doc.get_object(dest_pages_id)? {
            PdfObject::Dictionary(d) => d,
            _ => {
                return Err(PdfError::TypeMismatch {
                    id: dest_pages_id.number,
                    gen: dest_pages_id.generation,
                    expected: "Dictionary",
                    found: "Non-dictionary Pages root",
                })
            }
        };

        let mut kids = dest_pages_dict
            .get("Kids")
            .and_then(|k| k.as_array())
            .map(|k| k.to_vec())
            .unwrap_or_default();
        kids.push(PdfObject::Reference(dest_page_id));
        dest_pages_dict.insert("Kids", PdfObject::Array(kids));

        let count = dest_pages_dict
            .get("Count")
            .and_then(|c| c.as_i64())
            .unwrap_or(0);
        dest_pages_dict.insert("Count", count + 1);

        dest_doc.set_object(dest_pages_id, PdfObject::Dictionary(dest_pages_dict));

        Ok(dest_page_id)
    }

    /// Recursively clones a `PdfObject` from `src_doc` into `dest_doc`.
    pub fn clone_object(
        &mut self,
        obj: &PdfObject,
        src_doc: &mut PdfDocument,
        dest_doc: &mut PdfDocument,
    ) -> PdfResult<PdfObject> {
        match obj {
            PdfObject::Null => Ok(PdfObject::Null),
            PdfObject::Boolean(b) => Ok(PdfObject::Boolean(*b)),
            PdfObject::Integer(i) => Ok(PdfObject::Integer(*i)),
            PdfObject::Real(r) => Ok(PdfObject::Real(*r)),
            PdfObject::Name(n) => Ok(PdfObject::Name(n.clone())),
            PdfObject::String(s) => Ok(PdfObject::String(s.clone())),
            PdfObject::Array(arr) => {
                let mut cloned_arr = Vec::with_capacity(arr.len());
                for item in arr.iter() {
                    cloned_arr.push(self.clone_object(item, src_doc, dest_doc)?);
                }
                Ok(PdfObject::Array(cloned_arr))
            }
            PdfObject::Dictionary(dict) => {
                let mut cloned_dict = PdfDictionary::new();
                for (k, v) in &dict.0 {
                    let cloned_v = self.clone_object(v, src_doc, dest_doc)?;
                    cloned_dict.insert(k.clone(), cloned_v);
                }
                Ok(PdfObject::Dictionary(cloned_dict))
            }
            PdfObject::Stream(stream) => {
                let mut cloned_dict = PdfDictionary::new();
                for (k, v) in &stream.dict.0 {
                    let cloned_v = self.clone_object(v, src_doc, dest_doc)?;
                    cloned_dict.insert(k.clone(), cloned_v);
                }
                Ok(PdfObject::Stream(PdfStream {
                    dict: cloned_dict,
                    content: stream.content.clone(),
                }))
            }
            PdfObject::Reference(src_id) => {
                // If it points to source Catalog or Pages, map to destination counterparts
                if Some(*src_id) == self.src_catalog_id {
                    if let Some(dest_cat) = dest_doc.catalog_id() {
                        return Ok(PdfObject::Reference(dest_cat));
                    }
                }
                if Some(*src_id) == self.src_pages_id {
                    if let Ok(dest_pages) = dest_doc.pages_id() {
                        return Ok(PdfObject::Reference(dest_pages));
                    }
                }

                // If already cloned, reuse mapped destination ID
                if let Some(&dest_id) = self.id_map.get(src_id) {
                    return Ok(PdfObject::Reference(dest_id));
                }

                // Guard against cycles
                if !self.active_stack.insert(*src_id) {
                    return Err(PdfError::CircularReference {
                        id: src_id.number,
                        gen: src_id.generation,
                    });
                }

                // Allocate destination ID and register before recursing to handle recursive graphs
                let dest_id = dest_doc.alloc_object_id();
                dest_doc.set_object(dest_id, PdfObject::Null);
                self.id_map.insert(*src_id, dest_id);

                let src_referenced_obj = src_doc.get_object(*src_id)?;
                let cloned_referenced_obj =
                    self.clone_object(&src_referenced_obj, src_doc, dest_doc)?;

                dest_doc.set_object(dest_id, cloned_referenced_obj);
                self.active_stack.remove(src_id);

                Ok(PdfObject::Reference(dest_id))
            }
        }
    }
}
