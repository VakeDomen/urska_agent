use regex::RegexBuilder;

use crate::rag::{
    comm::{question::Question, OllamaClient},
    models::{
        chunks::{Chunk, HypeChunk},
        ChunkedFile,
    },
};

use super::summarize::summarize_document;

pub async fn hype(file: ChunkedFile<Chunk>, ollama: &OllamaClient) -> ChunkedFile<HypeChunk> {
    let summary = summarize_document(&file, ollama).await;
    let hype_question_prompts = generate_hype_prompt_questions(summary, &file);
    let hype_questions = ollama.answer_all(hype_question_prompts).await;
    let hype_chunks = generate_hype_chunks(&file.chunks, hype_questions);
    replace_chunks(file, hype_chunks)
}

fn replace_chunks(file: ChunkedFile<Chunk>, hype_chunks: Vec<HypeChunk>) -> ChunkedFile<HypeChunk> {
    let ChunkedFile {
        file_type,
        chunks: _,
        internal_id,
        tags,
        original_file_description,
        syntetic_file_description,
    } = file;

    ChunkedFile {
        file_type,
        chunks: hype_chunks,
        internal_id,
        tags,
        original_file_description,
        syntetic_file_description,
    }
}

fn generate_hype_chunks(chunks: &[Chunk], hype_questions: Vec<String>) -> Vec<HypeChunk> {
    let list_pattern = RegexBuilder::new(r"^\s*[\-\*]|\s*\d+\.\s*|\s*[a-zA-Z]\)\s*|\s*\(\d+\)\s*|\s*\([a-zA-Z]\)\s*|\s*\([ivxlcdm]+\)\s*")
        .case_insensitive(true)
        .build()
        .unwrap();

    let mut hype_chunks = vec![];
    for (i, chunk) in chunks.into_iter().enumerate() {
        let questions: Vec<String> = hype_questions[i]
            .split('\n')
            .map(|line| {
                let without_pattern = list_pattern.replace(line, "");
                without_pattern.trim().to_string()
            })
            .filter(|cleaned_line| !cleaned_line.is_empty())
            .collect();

        let hype_chunk = HypeChunk::from(chunk).set_questions(questions);
        hype_chunks.push(hype_chunk);
    }
    hype_chunks
}

fn generate_hype_prompt_questions(summary: String, file: &ChunkedFile<Chunk>) -> Vec<Question> {
    let question = format!("/no_think You will be given a passage from a document, that talks about: {}\n Your task is to analyze the context text (passage) and \
        generate essential questions that, when answered, capture the main points and core meaning of the text. \
        The questions should be exhaustive and understandable without context. When possible, named entities should be referenced by their full name. \
        However add questions that are diverse in topic. \
        It is extremely important that you only answer with questions and each question should be written in its own line (separated by newline) with no prefix.\
        And finally the answer to each question has to be found in the final context passage.", 
        summary);
    let system_prompt = "You are an agent specialized to only answer in form of questions.";

    file.chunks
        .iter()
        .map(|c| {
            Question::from(question.clone()).set_body(format!(r#"
            You will be given a chunk of text relating in some way to UP FAMNIT (University of Primorska - \
        Faculty of Mathematics, Natural Sciences, and Information Technologies). 


        Analyze the input text and generate all questions a student could ask that can be answered by the contents of the text. 
        It's important that the questions be exhaustive and understandable without context. 
        Named entities should always be referenced by their full name or short versions (like FAMNIT), but always referenced. 
        Only answer with questions, where each question should be written on its own line (separated by newline) with prefix: -. 
        It is especially important to generate only questions (many questions) when the text contains a table.
        If a question regards a person, a study program, or a particular year, always state the full name/information 
        in the question, especially regarding study programs. Start with most obvious simple questions and slowly ramp up in complexity.
        Make sure to exhaust all questions.



        -------------- Example Output to follow after </think>: --------------
        Who is Jernej Vičič?
        What is Eduroam and who can use it?
        What year is the class Applied Statistics offered?
        Who teaches Programming III (3) course?
        Who teaches Analysis III (3) – Functions of Many Variables course?
        How many internally selected elective courses do I have to choose in second year of Computer Science Bachlors?
        What courses are offered in the second year of computer science bachelor's?
        How many elective courses can I select in the third year of bachelor's?
        What is the typical class size for undergraduate courses in Computer Science?
        How is the academic year structured (semesters, exams, etc.)?
        Are there any student organizations or clubs related to Computer Science?
        What kind of career support does UP FAMNIT offer to its students?
        Is it possible to take an internship abroad during the Bachelor's program?
        Where can I find a table of courses offered in the masters program of Bioinformatics?
        Where can I find the official link to the master's thesis guidelines for Mathematical Sciences and Computer Science at UP FAMNIT?
        Where can I find the procedure for submitting a master’s thesis at UP FAMNIT?
        What are the requirements for obtaining a Bachelor’s degree in Computer Science at UP FAMNIT?
        What are the accommodation options for students at UP FAMNIT?
        What are the tuition fees for international students?
        Is there any financial aid or scholarship opportunities available?
        Is there a language requirement for international students?
        What is the grading system used at UP FAMNIT?
        How can I apply for a Bachelor’s program at UP FAMNIT?
        What are the deadlines for applying to the Bachelor’s programs?
        Is there an entrance exam for the Bachelor’s programs?



        -------------- Text to ask about: START -------------- 

        {:#?}

        -------------- Text to ask about: END -------------- 


        -------------- Additional notes: --------------
        * Always state the full name/information (e.g. 2nd year bachlors Computer Science)
        * Always adress the information itself and not the document
        * Often will information be found in the document link (education/master/computer/science -> Computer Science Masters programm)
        * If applicable always accompany the program with the year
        * Only anser with a sequence of questions and no additional text. First Question should start with "What..."
        * Speak in first persion (e.g. How many elective courses must I select in second year of Computer Science Bachlors)
        * Only ask about the text in the "Text to ask about" block
        * Exhaust all possible questions (the more the better)


        -------------- Output: -------------- 
    """
    
    system_msg = """
    /no_think 
    You are an AI that only answers in questions based on provided content \
    from UP FAMNIT (University of Primorska - Faculty of Mathematics, Natural Sciences, and \
    Information Technologies). Your task is to extract all possible questions from a given \
    text that a student might ask.
            "#, c))
        })
        .collect()
}
