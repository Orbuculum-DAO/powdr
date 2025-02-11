use std::marker::PhantomData;

use ast::asm_analysis::{
    AnalysisASMFile, BatchMetadata, FunctionStatement, Incompatible, IncompatibleSet, Machine,
};
use itertools::Itertools;
use number::FieldElement;

pub fn batch<T: FieldElement>(file: AnalysisASMFile<T>) -> AnalysisASMFile<T> {
    RomBatcher::default().batch(file)
}

#[derive(Default)]
struct Batch<'a, T> {
    statements: Vec<&'a FunctionStatement<T>>,
}

impl<'a, T: FieldElement> Batch<'a, T> {
    fn from_statement(s: &'a FunctionStatement<T>) -> Batch<T> {
        Batch {
            statements: vec![s],
        }
    }

    /// Returns true iff this batch consists exclusively of labels
    fn is_only_labels(&self) -> bool {
        self.statements
            .iter()
            .all(|s| matches!(s, FunctionStatement::Label(..)))
    }

    /// Returns true iff this batch contains at least one label
    fn contains_labels(&self) -> bool {
        self.statements
            .iter()
            .any(|s| matches!(s, FunctionStatement::Label(..)))
    }

    fn try_absorb(
        &mut self,
        s: &'a FunctionStatement<T>,
    ) -> Result<(), (&'a FunctionStatement<T>, IncompatibleSet)> {
        let batch = Self::from_statement(s);
        self.try_join(batch)
            .map_err(|(b, incompatible)| (b.statements.into_iter().next().unwrap(), incompatible))
    }

    fn try_join(&mut self, other: Self) -> Result<(), (Self, IncompatibleSet)> {
        match (self.is_only_labels(), other.contains_labels()) {
            // we can join any batch full of labels (in particular, an empty batch) with any batch
            (true, _) => {
                self.statements.extend(other.statements);
                Ok(())
            }
            // we cannot join a batch which doesn't only have labels with a batch which contains a label
            (false, true) => Err((other, IncompatibleSet([Incompatible::Label].into()))),
            // other types of batching are unimplemented
            (false, false) => Err((other, IncompatibleSet([Incompatible::Unimplemented].into()))),
        }
    }
}

#[derive(Default)]
struct RomBatcher<T> {
    marker: PhantomData<T>,
}

impl<T: FieldElement> RomBatcher<T> {
    // split a list of statements into compatible batches
    fn extract_batches(&self, machine_name: &str, machine: &mut Machine<T>) {
        if let Some(rom) = machine.rom.as_mut() {
            let batches: Vec<_> = rom
                .statements
                .iter()
                .peekable()
                .batching(|it| {
                    let mut batch = Batch::default();
                    loop {
                        // look at the next statement
                        match it.peek() {
                            // try to add it to this batch
                            Some(new_s) => match batch.try_absorb(new_s) {
                                Ok(()) => {
                                    it.next().unwrap();
                                }
                                Err((_, reason)) => {
                                    let res = BatchMetadata {
                                        size: batch.statements.len(),
                                        reason: Some(reason),
                                    };
                                    break Some(res);
                                }
                            },
                            None => {
                                break match batch.statements.len() {
                                    0 => None,
                                    _ => Some(BatchMetadata {
                                        size: batch.statements.len(),
                                        reason: None,
                                    }),
                                }
                            }
                        }
                    }
                })
                .collect();

            let lines_before = batches.iter().map(BatchMetadata::size).sum::<usize>();
            let lines_after = batches.len();

            log::debug!(
                "Batching complete for machine {} with savings of {}% in execution trace lines",
                machine_name,
                match lines_before {
                    0 => 0.,
                    lines_before => (1. - lines_after as f32 / lines_before as f32) * 100.,
                }
            );

            rom.batches = Some(batches);
        }
    }

    pub fn batch(&mut self, mut asm_file: AnalysisASMFile<T>) -> AnalysisASMFile<T> {
        for (name, machine) in asm_file.machines.iter_mut() {
            self.extract_batches(name, machine);
        }

        asm_file
    }
}

#[cfg(test)]
mod tests {

    use std::{fs, path::PathBuf};

    use number::GoldilocksField;
    use pretty_assertions::assert_eq;
    use test_log::test;

    use crate::{batcher, macro_expansion, romgen, type_check};

    fn test_batching(path: &str) {
        let base_path = PathBuf::from("../test_data/asm/batching");
        let file_name = base_path.join(path);
        let contents = fs::read_to_string(&file_name).unwrap();
        let parsed = parser::parse_asm::<GoldilocksField>(
            Some(file_name.as_os_str().to_str().unwrap()),
            &contents,
        )
        .unwrap();
        let expanded = macro_expansion::expand(parsed);
        let checked = type_check::check(expanded).unwrap();
        let rommed = romgen::generate_rom(checked);
        let batched = batcher::batch(rommed);

        assert_eq!(
            format!("{batched}")
                .replace("\n\n", "\n")
                .replace('\t', "    "),
            contents.replace("\n\n", "\n").replace('\t', "    "),
        );
    }

    #[test]
    fn labels() {
        test_batching("labels.asm")
    }
}
